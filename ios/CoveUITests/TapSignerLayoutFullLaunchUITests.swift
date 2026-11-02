import XCTest

/// Walks every TAPSIGNER setup and import screen at the largest accessibility text size and checks
/// that each screen's text and actions fit the window or scroll into it, through the debug
/// TAPSIGNER seam and an emulated card (docs/testing.md)
///
/// XCUITest reports a text's whole label even when it draws truncated, so the checks catch text and
/// actions pushed off screen or out of reach, not an ellipsis inside a label. Needs the hardware
/// signer host, which `xtask ios-ui` (`just ios-ui-background`) starts for every run
final class TapSignerLayoutFullLaunchUITests: XCTestCase {
    private static let esploraUrl = "http://127.0.0.1:39001"
    private static let largestContentSize = "UICTContentSizeCategoryAccessibilityXXXL"
    private static let newCvc = "246810"

    /// Drops the card's session once the operation has started on it, so the app offers to continue
    /// instead of failing outright
    private static let exchangesBeforeDrop = 3

    private var app: XCUIApplication!
    private var signer: HardwareSignerControl!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        signer = HardwareSignerControl()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launchEnvironment["COVE_UI_TEST_TAPSIGNER_BRIDGE"] = HardwareSignerControl.tapSignerBridge
    }

    /// A new card walks choose, advanced, the three PIN screens, setup retry and setup success
    func testTapSignerSetupScreensFitAtLargestTextSize() throws {
        try openTapSignerAtLargestTextSize(needsSetup: true, expectingAlert: "Setup TAPSIGNER?")
        try setUpCardThroughEveryScreen()
    }

    /// A card that already has a key walks the CVC screen, import retry and import success
    ///
    /// The emulator's seed fixes both its card identity and the root certificate the debug build
    /// trusts, so a set-up card is the same card the setup test made and would open its wallet;
    /// each test starts from a fresh install instead
    func testTapSignerImportScreensFitAtLargestTextSize() throws {
        try openTapSignerAtLargestTextSize(needsSetup: false, expectingAlert: "Import TAPSIGNER?")
        try importCardThroughEveryScreen()
    }

    /// Finishes onboarding at the default size, since Add Wallet needs a wallet and only the
    /// TAPSIGNER screens are under test, then relaunches at the largest size and scans the card
    private func openTapSignerAtLargestTextSize(needsSetup: Bool, expectingAlert alert: String) throws {
        try signer.requireHealthy()
        try signer.startTapSigner(needsSetup: needsSetup)

        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.terminate()

        app.launchArguments += ["-UIPreferredContentSizeCategoryName", Self.largestContentSize]
        app.launch(expecting: app.element("selectedWallet.balance"))
        app.scanTapSignerFromAddWallet(expectingAlert: alert)
    }

    // MARK: - Setup

    private func setUpCardThroughEveryScreen() throws {
        assertScreenFits(
            "choose chain code",
            texts: ["Setup Chain Code"],
            buttons: [button("Cancel"), button(startingWith: "Automatic Setup"), button("Advanced Setup")]
        )
        reachAndTap(button("Advanced Setup"), "choose chain code: Advanced Setup")

        assertScreenFits(
            "advanced setup",
            texts: ["Advanced Setup"],
            buttons: [button("Back"), button("Generate new string for me"), button("Continue")]
        )
        reachAndTap(button("Generate new string for me"), "advanced setup: Generate")
        reachAndTap(button("Continue"), "advanced setup: Continue")

        try enterCvc(HardwareSignerControl.tapSignerCvc, on: "factory CVC", title: "Enter Factory CVC", cancel: "Back")
        try enterCvc(Self.newCvc, on: "new CVC", title: "Create New CVC", cancel: "Back")

        // the card leaves the field once setup has started on it
        try signer.dropNextTapSignerSession(afterExchanges: Self.exchangesBeforeDrop)
        try enterCvc(Self.newCvc, on: "confirm CVC", title: "Confirm New CVC", cancel: "Back")

        let complete = app.staticTexts["Setup Complete"]
        XCTAssertTrue(
            button("Continue Setup").firstMatch.waitForExistence(timeout: 30),
            "the dropped session should leave setup in progress"
        )
        assertScreenFits("setup retry", texts: ["Setup in Progress"], buttons: [button("Cancel"), button("Continue Setup")])

        // each scan can stop at another stage, which offers to continue again
        for _ in 0 ..< 3 where !complete.exists {
            reachAndTap(button("Continue Setup"), "setup retry: Continue Setup")
            _ = complete.waitForExistence(timeout: 30)
        }

        assertScreenFits(
            "setup success",
            texts: ["Setup Complete"],
            buttons: [button("Cancel"), button(startingWith: "Download Backup"), button("Continue")]
        )
        reachAndTap(button("Continue"), "setup success: Continue")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    // MARK: - Import

    private func importCardThroughEveryScreen() throws {
        try signer.dropNextTapSignerSession(afterExchanges: Self.exchangesBeforeDrop)
        try enterCvc(HardwareSignerControl.tapSignerCvc, on: "import CVC", title: "Enter TAPSIGNER CVC", cancel: "Cancel")

        XCTAssertTrue(button("Retry").firstMatch.waitForExistence(timeout: 30), "the dropped session should offer to retry the import")
        assertScreenFits("import retry", texts: ["Could not complete import"], buttons: [button("Cancel"), button("Retry")])
        reachAndTap(button("Retry"), "import retry: Retry")

        XCTAssertTrue(app.staticTexts["Import Complete"].waitForExistence(timeout: 30))
        assertScreenFits("import success", texts: ["Import Complete"], buttons: [button("Cancel"), button("Continue")])
        reachAndTap(button("Continue"), "import success: Continue")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    // MARK: - CVC screens

    /// Checks the CVC screen fits with the number pad up, then enters the CVC and submits it
    private func enterCvc(_ cvc: String, on screen: String, title: String, cancel: String) throws {
        let field = app.secureTextFields.matching(identifier: "Enter CVC")
        XCTAssertTrue(app.staticTexts[title].waitForExistence(timeout: 15), "the \(screen) screen never showed")
        assertScreenFits(screen, texts: [title], buttons: [button(cancel), field, button("Continue")])

        reachAndTap(field, "\(screen): CVC field").typeText(cvc)
        reachAndTap(button("Continue"), "\(screen): Continue")
        XCTAssertTrue(app.staticTexts[title].waitForNonExistence(timeout: 15), "the \(screen) screen didn't accept the CVC")
    }

    // MARK: - Layout checks

    /// Every text and action must lie wholly inside the window once scrolled to, and the actions
    /// must take a tap there
    private func assertScreenFits(_ screen: String, texts: [String], buttons: [XCUIElementQuery]) {
        for text in texts {
            assertReachable(app.staticTexts.matching(NSPredicate(format: "label == %@", text)), "\(screen): '\(text)'", needsHit: false)
        }

        for button in buttons {
            assertReachable(button, "\(screen): \(button.firstMatch.description)", needsHit: true)
        }
    }

    @discardableResult
    private func reachAndTap(_ query: XCUIElementQuery, _ name: String) -> XCUIElement {
        let element = assertReachable(query, name, needsHit: true)
        element.tap()
        return element
    }

    /// Scrolls the screen until a match is on it, as a user would, and fails if none ever is
    ///
    /// Matches resolve afresh on every check: the sheet slides in over screens that keep their own
    /// Cancel and Back, and the sidebar keeps its buttons laid out off screen to the left, so a
    /// match picked once can be one the user never sees
    @discardableResult
    private func assertReachable(_ query: XCUIElementQuery, _ name: String, needsHit: Bool) -> XCUIElement {
        XCTAssertTrue(query.firstMatch.waitForExistence(timeout: 15), "\(name) is missing")

        // a sheet still sliding in takes no taps yet
        let settled = Date().addingTimeInterval(3)
        while reachableMatch(query, needsHit: needsHit) == nil, Date() < settled {
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        // scroll toward a match that scrolling could bring on screen; a downward swipe only
        // scrolls content back, since it runs only when that match sits above the top
        for _ in 0 ..< 8 where reachableMatch(query, needsHit: needsHit) == nil {
            let window = app.windows.firstMatch.frame
            let target = visibleScrollView() ?? app.windows.firstMatch

            if scrollCandidate(query).frame.minY < window.minY {
                target.swipeDown(velocity: .slow)
            } else {
                target.swipeUp(velocity: .slow)
            }
        }

        if let element = reachableMatch(query, needsHit: needsHit) {
            return element
        }

        let matches = query.allElementsBoundByIndex.map { "\($0.frame) hittable \($0.isHittable)" }
        let keyboard = app.keyboards.firstMatch
        XCTFail(
            "\(name) never fit on screen: matches \(matches) in window \(app.windows.firstMatch.frame), keyboard \(keyboard.exists ? "\(keyboard.frame)" : "none")"
        )
        return query.firstMatch
    }

    // MARK: - Queries

    private func button(_ label: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label == %@", label))
    }

    private func button(startingWith prefix: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", prefix))
    }

    private func reachableMatch(_ query: XCUIElementQuery, needsHit: Bool) -> XCUIElement? {
        query.allElementsBoundByIndex.first { isOnScreen($0, needsHit: needsHit) }
    }

    /// The match to scroll toward: one across the window but above, below or under the keyboard,
    /// rather than one already inside the window that something covers, which no scroll uncovers
    private func scrollCandidate(_ query: XCUIElementQuery) -> XCUIElement {
        let window = app.windows.firstMatch.frame
        let across = query.allElementsBoundByIndex.filter { element in
            let midX = element.frame.midX
            return midX > window.minX && midX < window.maxX
        }

        return across.first { !window.contains($0.frame) || isUnderKeyboard($0.frame) } ?? across.first ?? query.firstMatch
    }

    /// The smallest scroll view showing in the window, which is the sheet's rather than the
    /// full-screen wallet behind it
    private func visibleScrollView() -> XCUIElement? {
        let window = app.windows.firstMatch.frame
        let area = { (element: XCUIElement) -> CGFloat in
            let visible = element.frame.intersection(window)
            return visible.isNull ? 0 : visible.width * visible.height
        }

        return app.scrollViews.allElementsBoundByIndex.filter { area($0) > 0 }.min { area($0) < area($1) }
    }

    private func isOnScreen(_ element: XCUIElement, needsHit: Bool) -> Bool {
        guard element.exists else { return false }

        let frame = element.frame
        let window = app.windows.firstMatch.frame
        guard !frame.isEmpty, window.contains(frame), !isUnderKeyboard(frame) else { return false }

        return !needsHit || element.isHittable
    }

    private func isUnderKeyboard(_ frame: CGRect) -> Bool {
        let keyboard = app.keyboards.firstMatch
        return keyboard.exists && keyboard.frame.intersects(frame)
    }
}
