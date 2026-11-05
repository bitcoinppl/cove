import XCTest

/// Walks every TAPSIGNER setup and import screen at the largest accessibility text size and checks
/// that each screen's text and actions fit the window or scroll into it, through the debug
/// TAPSIGNER seam and an emulated card (docs/testing.md)
///
/// The checks catch text and actions pushed off screen, cut off by their scroll view or out of
/// reach. XCUITest reports a text's whole label even when it draws truncated, so each title and
/// button label also carries the debug truncation probe, which reports whether it draws whole.
/// Needs the hardware signer host, which `xtask ios-ui` (`just ios-ui-background`) starts for
/// every run
final class TapSignerLayoutFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let largestContentSize = "UICTContentSizeCategoryAccessibilityXXXL"
    private static let newCvc = "246810"

    /// The value a probed text reports when it draws whole, from `UITestTruncationProbe` in the app
    private static let fitsValue = "fits"

    /// Inside the screens' 16 to 20 point side padding, so a drag there lands on no field
    private static let dragMargin: CGFloat = 8

    /// Taller than the keyboard's suggestion bar and shorter than any screen's scroll area
    private static let rowHeight: CGFloat = 100

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
        app.launchEnvironment["COVE_UI_TEST_TRUNCATION_PROBE"] = "1"
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
            descriptions: ["A chain code works", "You can provide your own chain code"],
            buttons: [button("Cancel"), button(startingWith: "Automatic Setup"), button("Advanced Setup")]
        )
        reachAndTap(button("Advanced Setup"), "choose chain code: Advanced Setup")

        // the field grows vertically, which XCUITest can report as a text view rather than a text field
        let chainCodeField = app.descendants(matching: .any).matching(NSPredicate(
            format: "identifier == %@ AND (elementType == %lu OR elementType == %lu)",
            "tapSigner.chainCode",
            XCUIElement.ElementType.textField.rawValue,
            XCUIElement.ElementType.textView.rawValue
        ))
        let advancedContinue = button("Continue")
        assertScreenFits(
            "advanced setup",
            texts: ["Advanced Setup"],
            descriptions: ["Enter your custom 32-byte chain code"],
            buttons: [button("Back"), chainCodeField, button("Generate new string for me"), advancedContinue],
            awaitingInput: [advancedContinue]
        )
        reachAndTap(chainCodeField, "advanced setup: chain code field").typeText(Self.randomChainCode())
        reachAndTap(button("Continue"), "advanced setup: Continue")

        try enterCvc(
            HardwareSignerControl.tapSignerCvc,
            on: "factory CVC",
            title: "Enter Factory CVC",
            description: "The factory code is the 6 digit",
            cancel: "Back"
        )
        try enterCvc(Self.newCvc, on: "new CVC", title: "Create New CVC", description: "The CVC prevents unauthorized access", cancel: "Back")

        // the card leaves the field once setup has started on it
        try signer.dropNextTapSignerSession(afterExchanges: Self.exchangesBeforeDrop)
        try enterCvc(Self.newCvc, on: "confirm CVC", title: "Confirm New CVC", description: "Enter the same CVC again", cancel: "Back")

        let complete = app.staticTexts["Setup Complete"]
        XCTAssertTrue(
            button("Continue Setup").firstMatch.waitForExistence(timeout: 30),
            "the dropped session should leave setup in progress"
        )
        assertScreenFits(
            "setup retry",
            texts: ["Setup in Progress"],
            descriptions: ["Your TAPSIGNER saved its setup progress"],
            buttons: [button("Cancel"), button("Continue Setup")]
        )

        // each scan can stop at another stage, which offers to continue again
        for _ in 0 ..< 3 where !complete.exists {
            reachAndTap(button("Continue Setup"), "setup retry: Continue Setup")
            _ = complete.waitForExistence(timeout: 30)
        }

        assertScreenFits(
            "setup success",
            texts: ["Setup Complete"],
            descriptions: ["Your TAPSIGNER ready to use", "If you haven"],
            buttons: [button("Cancel"), button(startingWith: "Download Backup"), button("Continue")]
        )
        reachAndTap(button("Continue"), "setup success: Continue")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    // MARK: - Import

    private func importCardThroughEveryScreen() throws {
        try signer.dropNextTapSignerSession(afterExchanges: Self.exchangesBeforeDrop)
        try enterCvc(
            HardwareSignerControl.tapSignerCvc,
            on: "import CVC",
            title: "Enter TAPSIGNER CVC",
            description: "For security purposes, you need to enter your TAPSIGNER PIN",
            cancel: "Cancel"
        )

        XCTAssertTrue(button("Retry").firstMatch.waitForExistence(timeout: 30), "the dropped session should offer to retry the import")
        assertScreenFits(
            "import retry",
            texts: ["Could not complete import"],
            descriptions: ["Please try again and hold your TAPSIGNER steady"],
            buttons: [button("Cancel"), button("Retry")]
        )
        reachAndTap(button("Retry"), "import retry: Retry")

        XCTAssertTrue(app.staticTexts["Import Complete"].waitForExistence(timeout: 30))
        assertScreenFits(
            "import success",
            texts: ["Import Complete"],
            descriptions: ["Your TAPSIGNER ready to use"],
            buttons: [button("Cancel"), button("Continue")]
        )
        reachAndTap(button("Continue"), "import success: Continue")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    // MARK: - CVC screens

    /// Checks the CVC screen fits with the number pad up, then enters the CVC and submits it
    private func enterCvc(_ cvc: String, on screen: String, title: String, description: String, cancel: String) throws {
        let field = app.secureTextFields.matching(identifier: "Enter CVC")
        let submit = button("Continue")
        XCTAssertTrue(app.staticTexts[title].waitForExistence(timeout: 15), "the \(screen) screen never showed")
        assertScreenFits(
            screen,
            texts: [title],
            descriptions: [description],
            buttons: [button(cancel), field, submit],
            awaitingInput: [submit]
        )

        reachAndTap(field, "\(screen): CVC field").typeText(cvc)
        reachAndTap(button("Continue"), "\(screen): Continue")
        XCTAssertTrue(app.staticTexts[title].waitForNonExistence(timeout: 15), "the \(screen) screen didn't accept the CVC")
    }

    // MARK: - Layout checks

    /// Every text and action must lie wholly inside the window once scrolled to, the actions must
    /// be enabled and take a tap there, and every text, description and button label must draw
    /// whole
    ///
    /// Descriptions are matched by the start of their text and only checked for truncation, since a
    /// long one can be taller than the window. Text fields carry no probe, so only buttons among the
    /// actions are checked for truncation. Actions in `awaitingInput` stay disabled until the screen
    /// gets its input, and `reachAndTap` checks them enabled when the test taps them
    private func assertScreenFits(
        _ screen: String,
        texts: [String],
        descriptions: [String],
        buttons: [XCUIElementQuery],
        awaitingInput: [XCUIElementQuery] = []
    ) {
        for text in texts {
            let name = "\(screen): '\(text)'"
            let element = assertReachable(app.staticTexts.matching(NSPredicate(format: "label == %@", text)), name, needsHit: false)
            assertDrawsWhole(element, name)
        }

        for prefix in descriptions {
            let name = "\(screen): '\(prefix)…'"
            let element = app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
            XCTAssertTrue(element.waitForExistence(timeout: 15), "\(name) is missing")
            assertDrawsWhole(element, name)
        }

        for button in buttons {
            let name = "\(screen): \(button.firstMatch.description)"
            let element = assertReachable(button, name, needsHit: true)
            if !awaitingInput.contains(where: { $0 === button }) {
                XCTAssertTrue(element.isEnabled, "\(name) is disabled")
            }

            if element.elementType == .button {
                assertDrawsWhole(element, name)
            }
        }
    }

    /// Fails unless the truncation probe of the text, or of the button's label, reports it draws
    /// whole
    ///
    /// A button takes its label's accessibility value. A text without the probe has no value, so
    /// dropping the probe fails here too. The probe measures after layout, so its value gets a
    /// moment to settle
    private func assertDrawsWhole(_ element: XCUIElement, _ name: String) {
        let value = { element.value as? String ?? "" }
        let settled = Date().addingTimeInterval(3)
        while value() != Self.fitsValue, Date() < settled {
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTAssertEqual(value(), Self.fitsValue, "\(name) doesn't draw whole: its truncation probe reports '\(value())'")
    }

    @discardableResult
    private func reachAndTap(_ query: XCUIElementQuery, _ name: String) -> XCUIElement {
        let element = assertReachable(query, name, needsHit: true)

        // a tap on a disabled button does nothing, which would only show as a later timeout
        XCTAssertTrue(element.isEnabled, "\(name) is disabled")
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

        // scroll toward a match that scrolling could bring on screen; dragging content down only
        // scrolls it back, since it runs only when that match sits above the top
        for _ in 0 ..< 8 where reachableMatch(query, needsHit: needsHit) == nil {
            let candidate = scrollCandidate(query)
            let target = scrollContainers(holding: candidate).last ?? visibleScrollView() ?? app.windows.firstMatch
            drag(target, revealingContentBelow: candidate.frame.minY >= visibleArea(of: candidate).minY)
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

    /// Drags the container's content along its leading margin, within the part of it showing
    /// above the keyboard, as a user would
    ///
    /// `swipeUp` starts at the container's center, which on a sheet with the number pad up lands
    /// on the keyboard's backdrop; that backdrop sits above the frame XCUITest reports for the
    /// keyboard, takes the drag, and the content never moves. The margin keeps the drag off the
    /// multi-line chain code field, which scrolls its own text instead of the screen
    private func drag(_ container: XCUIElement, revealingContentBelow: Bool) {
        let window = app.windows.firstMatch.frame
        var area = container.frame.intersection(window)
        if area.isNull { area = window }

        let keyboard = app.keyboards.firstMatch
        if keyboard.exists, keyboard.frame.minY > area.minY {
            area.size.height = min(area.height, keyboard.frame.minY - area.minY)
        }

        let point = { (fraction: CGFloat) in
            container.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(
                dx: area.minX + Self.dragMargin - container.frame.minX,
                dy: area.minY + area.height * fraction - container.frame.minY
            ))
        }

        let (start, end) = revealingContentBelow ? (point(0.7), point(0.3)) : (point(0.3), point(0.7))
        start.press(forDuration: 0.05, thenDragTo: end, withVelocity: .slow, thenHoldForDuration: 0.1)
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

        return across.first { !visibleArea(of: $0).contains($0.frame) || isUnderKeyboard($0.frame) } ?? across.first ?? query.firstMatch
    }

    /// The part of the window the element can show in: the window cut down to every scroll area
    /// that holds it
    ///
    /// XCUITest reports an element's whole frame, not the part its scroll view clips, so a title
    /// scrolled half under a sheet's edge still lies inside the window
    private func visibleArea(of element: XCUIElement) -> CGRect {
        scrollContainers(holding: element).reduce(app.windows.firstMatch.frame) { $0.intersection($1.frame) }
    }

    /// The scroll views, lists and tables that hold the element, outermost first
    private func scrollContainers(holding element: XCUIElement) -> [XCUIElement] {
        let frame = element.frame
        let type = element.elementType
        let label = element.label
        let containers = [app.scrollViews, app.collectionViews, app.tables].flatMap(\.allElementsBoundByIndex)

        return containers
            .filter { container in
                guard let snapshot = try? container.snapshot() else { return false }
                return snapshot.children.contains { holds($0, type: type, label: label, frame: frame) }
            }
            .sorted { $0.frame.width * $0.frame.height > $1.frame.width * $1.frame.height }
    }

    private func holds(_ snapshot: XCUIElementSnapshot, type: XCUIElement.ElementType, label: String, frame: CGRect) -> Bool {
        if snapshot.elementType == type, snapshot.label == label, isSameFrame(snapshot.frame, frame) { return true }

        return snapshot.children.contains { holds($0, type: type, label: label, frame: frame) }
    }

    /// The element and its snapshot report the same frame through separate reads, which can
    /// differ in the last bits, as 850.9999999999997 against 851
    private func isSameFrame(_ lhs: CGRect, _ rhs: CGRect) -> Bool {
        abs(lhs.minX - rhs.minX) < 0.5 && abs(lhs.minY - rhs.minY) < 0.5
            && abs(lhs.width - rhs.width) < 0.5 && abs(lhs.height - rhs.height) < 0.5
    }

    /// The smallest scroll view showing in the window, which is the sheet's rather than the
    /// full-screen wallet behind it
    ///
    /// Strips a row high are left out: the text keyboard's suggestion bar is a 44 point scroll
    /// view above the keyboard, and dragging it scrolls only the suggestions
    private func visibleScrollView() -> XCUIElement? {
        let window = app.windows.firstMatch.frame
        let area = { (element: XCUIElement) -> CGFloat in
            let visible = element.frame.intersection(window)
            return visible.isNull ? 0 : visible.width * visible.height
        }

        return app.scrollViews.allElementsBoundByIndex
            .filter { area($0) > 0 && $0.frame.height > Self.rowHeight }
            .min { area($0) < area($1) }
    }

    private func isOnScreen(_ element: XCUIElement, needsHit: Bool) -> Bool {
        guard element.exists else { return false }

        let frame = element.frame
        guard !frame.isEmpty, visibleArea(of: element).contains(frame), !isUnderKeyboard(frame) else { return false }

        return !needsHit || element.isHittable
    }

    private func isUnderKeyboard(_ frame: CGRect) -> Bool {
        let keyboard = app.keyboards.firstMatch
        return keyboard.exists && keyboard.frame.intersects(frame)
    }

    /// A random 32-byte chain code as hex, as a user typing their own would enter it
    private static func randomChainCode() -> String {
        (0 ..< 32).map { _ in String(format: "%02x", UInt8.random(in: 0 ... 255)) }.joined()
    }
}
