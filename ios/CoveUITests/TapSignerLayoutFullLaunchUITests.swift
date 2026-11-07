import XCTest

/// Walks every TAPSIGNER setup and import screen, and the CVC screen signing opens, at the largest
/// accessibility text size and checks that each screen's text and actions fit the window or scroll
/// into it, through the debug TAPSIGNER seam and an emulated card (docs/testing.md)
///
/// The checks catch text and actions pushed off screen, cut off by their scroll view or out of
/// reach. XCUITest reports a text's whole label even when it draws truncated, so each title and
/// button label also carries the debug truncation probe, which reports whether it draws whole.
/// Needs the hardware signer host, and the fake node for the signing test, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run
final class TapSignerLayoutFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let largestContentSize = "UICTContentSizeCategoryAccessibilityXXXL"
    private static let newCvc = "246810"
    private static let fundedSats: UInt64 = 100_000

    /// The value a probed text reports when it draws whole, from `UITestTruncationProbe` in the app
    private static let fitsValue = "fits"

    /// Inside the screens' 16 to 20 point side padding, so a drag there lands on no field
    private static let dragMargin: CGFloat = 8

    /// Taller than the keyboard's suggestion bar and shorter than any screen's scroll area, and the
    /// drag that moves a covered match toward the middle of the screen
    private static let rowHeight: CGFloat = 100

    /// About one line of body text at the largest accessibility size, the most of a description
    /// taller than its scroll area that must show to count its first or last line as on screen
    private static let lineHeight: CGFloat = 72

    /// About how far a touch moves before a scroll view takes it as a drag, added to each drag so
    /// the content moves about as far as asked
    private static let touchSlop: CGFloat = 10

    /// Room left between an element scrolled into view and the edge of the space it shows in
    private static let scrollRoom: CGFloat = 8

    /// Drops the card's session once the operation has started on it, so the app offers to continue
    /// instead of failing outright
    private static let exchangesBeforeDrop = 3

    private var app: XCUIApplication!
    private var signer: HardwareSignerControl!

    /// The CVC screen under check and where its number pad came to rest, which every drag on the
    /// screen must leave in place, so all its texts are checked in the space above the number pad
    private var numberPad: NumberPadWatch?

    private struct NumberPadWatch {
        let screen: String
        let frame: CGRect
        var drags = 0
    }

    private enum KeyboardState {
        case gone
        case settled(CGRect)
        case moving(CGRect)
    }

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

    /// A funded card's Sign using TAPSIGNER opens the CVC screen with signing's own message
    ///
    /// That screen shows before the app contacts the card, so signing failing on the card
    /// (#965) doesn't stop the check; the test cancels instead of submitting a CVC. The card is
    /// imported and funded at the default size, since only the signing screen is under test
    func testTapSignerSigningCvcScreenFitsAtLargestTextSize() throws {
        let node = FakeNodeControl()
        try node.requireHealthy()
        try node.reset()
        try signer.requireHealthy()
        try signer.startTapSigner()

        // the draft prices its fee from the fake node
        app.launchEnvironment["COVE_FEE_URL"] = "\(Self.esploraUrl)/api/v1/fees/recommended"
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()

        app.scanTapSignerFromAddWallet(expectingAlert: "Import TAPSIGNER?")
        app.enterTapSignerCvc(HardwareSignerControl.tapSignerCvc)
        XCTAssertTrue(app.staticTexts["Import Complete"].waitForExistence(timeout: 30))
        app.tapButton(named: "Continue")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))

        _ = try node.fund(address: app.copyReceiveAddress(using: signer), sats: [Self.fundedSats])
        app.refreshUntilBalance(sats: Self.fundedSats)
        app.terminate()

        // the app reopens the card's wallet, the one last selected
        app.launchArguments += ["-UIPreferredContentSizeCategoryName", Self.largestContentSize]
        app.launch(expecting: app.element("selectedWallet.balance"))
        app.assertBalance(sats: Self.fundedSats, timeout: 60)

        // at this size the send screen opens with the address focused just under the keyboard,
        // where only a drag brings it into reach
        reachAndTap(app.buttons.matching(NSPredicate(format: "label == 'Send' OR label ENDSWITH ', Send'")), "wallet: Send")
        reachAndTap(app.textViews.matching(identifier: "sendFlow.address"), "send: address").typeText(XCUIApplication.externalAddress)
        reachAndTap(app.textFields.matching(identifier: "sendFlow.amount"), "send: amount")
        app.tapMax()

        // the Next button shows once fee rates load, and the larger text can push it below the fold
        let next = app.buttons.matching(identifier: "sendFlow.next")
        XCTAssertTrue(next.firstMatch.waitForExistence(timeout: 60), "Next never showed on the send screen")
        reachAndTap(next, "send: Next")
        reachAndTap(button("Sign using TAPSIGNER"), "hardware send: Sign using TAPSIGNER")

        assertCvcScreenFits(
            on: "signing CVC",
            title: "Enter TAPSIGNER CVC",
            description: "For security purposes, you must enter your TAPSIGNER PIN before you can sign",
            cancel: "Cancel"
        )

        reachAndTap(button("Cancel"), "signing CVC: Cancel")
        XCTAssertTrue(app.staticTexts["Enter TAPSIGNER CVC"].waitForNonExistence(timeout: 15), "Cancel didn't close the signing CVC screen")
        XCTAssertTrue(button("Sign using TAPSIGNER").firstMatch.waitForExistence(timeout: 15), "Cancel should leave the draft open")
    }

    // MARK: - Setup

    private func setUpCardThroughEveryScreen() throws {
        assertScreenFits(
            "choose chain code",
            texts: ["Setup Chain Code"],
            descriptions: ["A chain code works", "You can provide your own chain code"],
            buttons: [button("Cancel"), button(startingWith: "Automatic Setup"), button("Advanced Setup")]
        )

        assertReportFits("tapSigner.automaticSetup.truncation", "choose chain code: Automatic Setup", probes: 2)
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

        assertReportFits("tapSigner.downloadBackup.truncation", "setup success: Download Backup", probes: 2)
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
        assertCvcScreenFits(on: screen, title: title, description: description, cancel: cancel)

        let field = app.secureTextFields.matching(identifier: "Enter CVC")
        reachAndTap(field, "\(screen): CVC field").typeText(cvc)
        reachAndTap(button("Continue"), "\(screen): Continue")
        XCTAssertTrue(app.staticTexts[title].waitForNonExistence(timeout: 15), "the \(screen) screen didn't accept the CVC")
    }

    /// Checks the CVC screen fits with the number pad up, before any CVC is entered
    private func assertCvcScreenFits(on screen: String, title: String, description: String, cancel: String) {
        let submit = button("Continue")
        XCTAssertTrue(app.staticTexts[title].waitForExistence(timeout: 15), "the \(screen) screen never showed")
        numberPad = NumberPadWatch(screen: screen, frame: settledNumberPad(on: screen))
        defer { numberPad = nil }

        assertScreenFits(
            screen,
            texts: [title],
            descriptions: [description],
            buttons: [button(cancel), app.secureTextFields.matching(identifier: "Enter CVC"), submit],
            awaitingInput: [submit]
        )

        // every drag checked the number pad, but a text that showed with no drag was checked
        // against the space above it too, so it must still be in place now
        assertNumberPadInPlace(after: "checking the screen")
        note("number pad on the \(screen) screen stayed in place through \(numberPad?.drags ?? 0) drags")
    }

    /// Waits for the number pad a CVC screen opens with to come to rest on screen, so its texts are
    /// checked against the space a user reads them in rather than the larger one before it opens,
    /// and returns where it rests
    private func settledNumberPad(on screen: String) -> CGRect {
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 15), "the \(screen) screen opened without its number pad")

        // the number pad slides in from below the window
        switch keyboardState(within: 10) {
        case let .settled(frame):
            return frame

        case let .moving(frame):
            XCTFail("the \(screen) screen's number pad never came to rest: it was last at \(frame) in window \(app.windows.firstMatch.frame)")

        case .gone:
            XCTFail("the \(screen) screen's number pad closed before it came to rest")
        }

        return .null
    }

    /// On a CVC screen, fails unless the number pad is still where it came to rest, so no text was
    /// checked against the larger space a closed or moved number pad leaves
    ///
    /// The screen's scroll view keeps its default keyboard dismissal, and drags on every CVC screen
    /// have left the number pad in place, so one that closes or moves it is a change to look at
    private func assertNumberPadInPlace(after action: String) {
        guard let watch = numberPad else { return }

        switch keyboardState(within: 3) {
        case let .settled(frame) where isSameFrame(frame, watch.frame):
            return

        case let .settled(frame):
            XCTFail("\(action) on the \(watch.screen) screen moved its number pad from \(watch.frame) to \(frame)")

        case let .moving(frame):
            XCTFail("\(action) on the \(watch.screen) screen left its number pad moving, last at \(frame)")

        case .gone:
            XCTFail("\(action) on the \(watch.screen) screen closed its number pad")
        }
    }

    /// Waits up to `timeout` for the keyboard to close, or to hold still inside the window, and
    /// returns how it was left
    ///
    /// A keyboard sliding away still exists until it has gone, so a frame read once can't tell it
    /// from one that stays
    private func keyboardState(within timeout: TimeInterval) -> KeyboardState {
        let keyboard = app.keyboards.firstMatch
        let window = app.windows.firstMatch.frame
        let deadline = Date().addingTimeInterval(timeout)
        var last = CGRect.null

        repeat {
            guard keyboard.exists else { return .gone }

            let frame = keyboard.frame
            if isSameFrame(frame, last), !frame.isEmpty, frame.minY < window.maxY { return .settled(frame) }

            last = frame
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        } while Date() < deadline

        return keyboard.exists ? .moving(last) : .gone
    }

    // MARK: - Layout checks

    /// Every text and action must lie wholly inside the window once scrolled to, the actions must
    /// be enabled and take a tap there, and every text, description and button label must draw
    /// whole
    ///
    /// Descriptions are matched by the start of their text. One that fits the space it shows in
    /// must scroll wholly into it, and a taller one must scroll its first and then its last line
    /// into it, as `assertReadable` checks. Text fields carry no probe, so only buttons among the
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
            let element = assertReadable(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)), name)
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
    /// A button joins the values of the texts in its label, so one holding a probed title and a
    /// probed description reads `fits, fits`, and every part must say `fits`. A text without the
    /// probe has no value, so dropping the probe fails here too. The probe measures after layout,
    /// so its value gets a moment to settle
    private func assertDrawsWhole(_ element: XCUIElement, _ name: String) {
        let value = { element.value as? String ?? "" }
        let fits = { value().components(separatedBy: ", ").allSatisfy { $0 == Self.fitsValue } }
        let settled = Date().addingTimeInterval(3)
        while !fits(), Date() < settled {
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTAssertTrue(fits(), "\(name) doesn't draw whole: its truncation probe reports '\(value())'")
    }

    /// Fails unless the truncation report beside a button holds `probes` probed texts and reads
    /// `fits`
    ///
    /// A button merges the texts of its label into one element, so a button holding a title and a
    /// description sums up both probes through an element beside it. The count catches one probe
    /// of the two dropped, which the summed value alone wouldn't
    private func assertReportFits(_ identifier: String, _ name: String, probes: Int) {
        let report = app.descendants(matching: .any).matching(identifier: identifier).firstMatch
        XCTAssertTrue(report.waitForExistence(timeout: 15), "\(name)'s truncation report is missing")
        XCTAssertEqual(report.label, "Truncation report of \(probes) texts", "\(name) should probe its title and its description")
        assertDrawsWhole(report, "\(name) title or description")
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

        // scroll a match that scrolling could bring on screen by about as far as it lies outside the
        // space above any keyboard, so content with less scroll range than a drag stops partway
        // instead of swinging between its ends; a match inside that space but covered, as by the
        // keyboard's toolbar, moves a row toward the middle
        for _ in 0 ..< 8 where reachableMatch(query, needsHit: needsHit) == nil {
            let candidate = scrollCandidate(query)
            let target = scrollContainers(holding: candidate).last ?? visibleScrollView() ?? app.windows.firstMatch
            let area = readableArea(of: candidate)
            let nudge = candidate.frame.midY > area.midY ? Self.rowHeight : -Self.rowHeight
            let distance = scrollDistance(showing: candidate.frame, in: area)
            drag(target, by: distance != 0 ? distance : nudge)
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

    /// Scrolls a description wholly on screen, or for one taller than the space it shows in, its
    /// first line and then its last, as a user reading it would, and fails if any never shows
    @discardableResult
    private func assertReadable(_ query: XCUIElementQuery, _ name: String) -> XCUIElement {
        let element = query.firstMatch
        XCTAssertTrue(element.waitForExistence(timeout: 15), "\(name) is missing")
        guard element.frame.height > readableArea(of: element).height else {
            return assertReachable(query, name, needsHit: false)
        }

        assertLineReachable(element, name, first: true)
        assertLineReachable(element, name, first: false)
        return element
    }

    /// Records a finding in the test log and the result bundle
    private func note(_ message: String) {
        print("TapSignerLayout: \(message)")
        XCTContext.runActivity(named: message) { _ in }
    }

    /// Scrolls until the first or last line of a description taller than its readable area lies
    /// inside that area, and fails if it never does
    ///
    /// A scroll moves the line only up or down, so it must also lie across the area from side to
    /// side, as one moved off the window sideways doesn't
    private func assertLineReachable(_ element: XCUIElement, _ name: String, first: Bool) {
        let which = first ? "first" : "last"
        let line = { () -> CGRect in
            let frame = element.frame
            return CGRect(x: frame.minX, y: first ? frame.minY : frame.maxY - Self.lineHeight, width: frame.width, height: Self.lineHeight)
        }

        let showsVertically = { () -> Bool in
            let area = self.readableArea(of: element)
            return line().minY >= area.minY - 0.5 && line().maxY <= area.maxY + 0.5
        }

        let showsAcross = { () -> Bool in
            let frame = element.frame
            let area = self.readableArea(of: element)
            return !frame.isEmpty && frame.minX >= area.minX - 0.5 && frame.maxX <= area.maxX + 0.5
        }

        for _ in 0 ..< 12 where !showsVertically() {
            let target = scrollContainers(holding: element).last ?? visibleScrollView() ?? app.windows.firstMatch
            drag(target, by: scrollDistance(showing: line(), in: readableArea(of: element)))
        }

        XCTAssertTrue(
            showsVertically() && showsAcross(),
            "\(name)'s \(which) line never showed: it spans \(element.frame) but can show only in \(readableArea(of: element)), with the keyboard \(app.keyboards.firstMatch.exists ? "up" : "closed")"
        )
    }

    /// How far to scroll so `frame` lies inside `area`, positive to bring content below into view,
    /// or zero when it already lies there from top to bottom
    private func scrollDistance(showing frame: CGRect, in area: CGRect) -> CGFloat {
        let room = max(0, min(Self.scrollRoom, (area.height - frame.height) / 2))
        if frame.maxY > area.maxY { return frame.maxY - area.maxY + room }
        if frame.minY < area.minY { return frame.minY - area.minY - room }

        return 0
    }

    /// The part of the window the element can show in and read, above the keyboard
    private func readableArea(of element: XCUIElement) -> CGRect {
        var area = visibleArea(of: element)
        let keyboard = app.keyboards.firstMatch
        if keyboard.exists, keyboard.frame.minY < area.maxY {
            area.size.height = max(0, keyboard.frame.minY - area.minY)
        }

        return area
    }

    /// Drags the container's content by about `distance` points, positive to bring content below
    /// into view, along its leading margin within the part of it showing above the keyboard, as a
    /// user would, and on a CVC screen checks the drag left the number pad in place
    ///
    /// A drag moves at most 40% of that part, and the caller drags again to go further, so a text
    /// that fits only partway through a short scroll range isn't swung past. `swipeUp` starts at the
    /// container's center, which on a sheet with the number pad up lands on the keyboard's
    /// backdrop; that backdrop sits above the frame XCUITest reports for the keyboard, takes the
    /// drag, and the content never moves. The margin keeps the drag off the multi-line chain code
    /// field, which scrolls its own text instead of the screen
    private func drag(_ container: XCUIElement, by distance: CGFloat) {
        let window = app.windows.firstMatch.frame
        var area = container.frame.intersection(window)
        if area.isNull { area = window }

        let keyboard = app.keyboards.firstMatch
        if keyboard.exists, keyboard.frame.minY > area.minY {
            area.size.height = min(area.height, keyboard.frame.minY - area.minY)
        }

        let step = max(-area.height * 0.4, min(area.height * 0.4, distance))
        let travel = step + (step < 0 ? -Self.touchSlop : Self.touchSlop)
        let point = { (y: CGFloat) in
            container.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(
                dx: area.minX + Self.dragMargin - container.frame.minX,
                dy: y - container.frame.minY
            ))
        }

        point(area.midY + travel / 2).press(forDuration: 0.05, thenDragTo: point(area.midY - travel / 2), withVelocity: .slow, thenHoldForDuration: 0.1)

        guard var watch = numberPad else { return }

        watch.drags += 1
        numberPad = watch
        assertNumberPadInPlace(after: "drag \(watch.drags)")
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
        guard !frame.isEmpty, shows(element, frame, in: visibleArea(of: element)), !isUnderKeyboard(frame) else { return false }

        return !needsHit || element.isHittable
    }

    /// The send screen's address editor, the one element allowed to start left of its scroll area
    private static let addressEditor = "sendFlow.address"

    /// How far left of its scroll area the address editor's frame may start:
    /// `ios/Cove/Flows/SendFlow/SetAmountScreen/AddressTextEditor.swift` draws the editor
    /// `.offset(x: -4)`, which moves 4 of its 5 points of line padding past the scroll view's side
    /// padding, so none of its text is cut off
    private static let addressEditorLeftOverhang: CGFloat = 4

    /// Slack for a frame XCUITest reports a fraction of a point off, as in `isSameFrame`
    private static let frameRounding: CGFloat = 0.5

    /// Whether the frame lies inside the area, or for the send screen's address editor, inside it
    /// but for starting up to `addressEditorLeftOverhang` plus `frameRounding` left of it
    ///
    /// XCUITest reports no bounds for a field's text, only its frame, so every other field, like
    /// every other element, must lie wholly inside the area
    private func shows(_ element: XCUIElement, _ frame: CGRect, in area: CGRect) -> Bool {
        guard element.elementType == .textView, element.identifier == Self.addressEditor else { return area.contains(frame) }

        // only the left edge moves, so the editor's right edge, top and bottom stay strict
        let overhang = Self.addressEditorLeftOverhang + Self.frameRounding
        let widened = CGRect(x: area.minX - overhang, y: area.minY, width: area.width + overhang, height: area.height)
        return widened.contains(frame) && app.windows.firstMatch.frame.contains(frame)
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
