import XCTest

/// Scrolls a screen as a user would and checks that its text and actions fit inside the window
///
/// The checks catch text and actions pushed off screen, cut off by their scroll view or out of
/// reach. XCUITest reports a text's whole label even when it draws truncated, so the fit checks
/// need the app launched with `COVE_UI_TEST_TRUNCATION_PROBE=1` (`truncationProbeEnvironment`),
/// and read whether each text draws whole from the value its truncation probe reports
struct ScreenReachability {
    /// Turns on the app's truncation probe, `UITestTruncationProbe`, for the launch
    static let truncationProbeEnvironment = ("COVE_UI_TEST_TRUNCATION_PROBE", "1")

    private static let fitsValue = "fits"
    /// What a probe reports for a text that doesn't draw whole: cut short, or past a side of the window
    private static let notWholeValues: Set<String> = ["truncated", "offscreen"]

    /// Inside the screens' 16 to 20 point side padding, so a drag there lands on no field
    private static let dragMargin: CGFloat = 8

    /// Taller than the keyboard's suggestion bar and shorter than any screen's scroll area
    private static let rowHeight: CGFloat = 100

    /// Frames are in points with fractional edges, and adjacent views can share a rounded edge
    private static let overlapTolerance: CGFloat = 1

    /// Room for one line of a text at the largest text size, about 50 points, with margin
    private static let endLineHeight: CGFloat = 80

    let app: XCUIApplication

    /// Every text and action must lie wholly inside the window once scrolled to, the actions must
    /// take a tap there, each text and each button's label must draw whole, no two of them may
    /// overlap, and no probed text on the screen may draw truncated
    ///
    /// `texts` match a text's whole label and `descriptions` the start of one, for a long
    /// paragraph. `fields` are selectable values drawn under their labels, such as a received
    /// message's, checked as `texts` are. `controls` are actions without a probed label, such as
    /// icon-only buttons and fields, checked for reach only
    func assertScreenFits(
        _ screen: String,
        texts: [String],
        descriptions: [String] = [],
        fields: [(label: String, value: String)] = [],
        buttons: [XCUIElementQuery],
        controls: [XCUIElementQuery] = []
    ) {
        let textQueries = (texts + fields.flatMap { [$0.label, $0.value] })
            .map { ("'\($0)'", app.staticTexts.matching(NSPredicate(format: "label == %@", $0))) }
            + descriptions.map { ("'\($0)…'", app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", $0))) }

        var shown: [(name: String, element: XCUIElement)] = []
        for (label, query) in textQueries {
            let name = "\(screen): \(label)"
            if let tall = tallerThanItsScrollArea(query) {
                assertBothEndsFit(tall, name)
                assertDrawsWhole(tall, name)
                shown.append((name, tall))
                continue
            }

            let text = assertReachable(query, name, needsHit: false)
            assertDrawsWhole(text, name)
            shown.append((name, text))
        }

        for button in buttons {
            let name = "\(screen): \(button.firstMatch.description)"
            let element = assertReachable(button, name, needsHit: true)
            assertDrawsWhole(element, name)
            shown.append((name, element))
        }

        assertNoOverlap(shown, fields: fields.map { ("\(screen): '\($0.label)'", "\(screen): '\($0.value)'") })

        for control in controls {
            assertReachable(control, "\(screen): \(control.firstMatch.description)", needsHit: true)
        }

        assertNoTruncatedText(screen)
    }

    /// Fails unless the truncation probe of the text, or of the button's label, reports that it
    /// draws whole
    ///
    /// A button takes its label's accessibility value. A text without the probe has no value, so
    /// dropping the probe fails here too. The probe measures after layout, so its value gets a
    /// moment to settle
    func assertDrawsWhole(_ element: XCUIElement, _ name: String) {
        let value = { element.value as? String ?? "" }
        let settled = Date().addingTimeInterval(3)
        while value() != Self.fitsValue, Date() < settled {
            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        }

        XCTAssertEqual(value(), Self.fitsValue, "\(name) doesn't draw whole: its truncation probe reports '\(value())'")
    }

    /// Fails when two of the elements' frames overlap
    ///
    /// The probe compares a text with its own laid-out size, so a text that keeps its full height
    /// inside a shorter parent that clips it still reports that it fits. Its accessibility frame
    /// keeps the full height, though, and runs into whatever follows the parent. Scrolling moves
    /// every frame, so they're all read at one scroll position
    ///
    /// Some iOS versions report a selectable text's frame standing out the same distance past the
    /// text on every side, so each field's value is first inset by how far its leading edge sits
    /// before its label's, which the view aligns with it. Where iOS adds nothing that measures 0
    private func assertNoOverlap(_ elements: [(name: String, element: XCUIElement)], fields: [(label: String, value: String)]) {
        var frames = elements.map { ($0.name, $0.element.frame) }
        for field in fields {
            guard let label = frames.first(where: { $0.0 == field.label })?.1,
                  let value = frames.firstIndex(where: { $0.0 == field.value })
            else { continue }

            let outset = max(0, label.minX - frames[value].1.minX)
            frames[value].1 = frames[value].1.insetBy(dx: outset, dy: outset)
        }

        for (index, (name, frame)) in frames.enumerated() {
            for (otherName, otherFrame) in frames[(index + 1)...] {
                let shared = frame.intersection(otherFrame)
                XCTAssertFalse(
                    shared.width > Self.overlapTolerance && shared.height > Self.overlapTolerance,
                    "\(name) at \(frame) overlaps \(otherName) at \(otherFrame)"
                )
            }
        }
    }

    /// Fails when any probed text or button label in the app reports that it draws truncated or
    /// past a side of the window, so a text the caller didn't list is checked too
    ///
    /// A probe reports truncated until it has measured, so the sweep gets a moment to settle
    func assertNoTruncatedText(_ screen: String) {
        var truncated: [String] = []
        let settled = Date().addingTimeInterval(3)
        repeat {
            truncated = (try? app.snapshot()).map(labelsNotDrawnWhole(in:)) ?? ["the app's accessibility tree could not be read"]
            if truncated.isEmpty { return }

            RunLoop.current.run(until: Date().addingTimeInterval(0.25))
        } while Date() < settled

        XCTFail("\(screen): text doesn't draw whole: \(truncated)")
    }

    private func labelsNotDrawnWhole(in snapshot: XCUIElementSnapshot) -> [String] {
        let value = snapshot.value as? String ?? ""
        let own = Self.notWholeValues.contains(value) ? ["\(snapshot.label) (\(value))"] : []
        return own + snapshot.children.flatMap(labelsNotDrawnWhole(in:))
    }

    /// A match too tall to scroll wholly into view by dragging, such as a revealed private key
    /// wrapped at the largest text size, which a user reads by scrolling through it
    ///
    /// A drag moves the content by 40% of its scroll area, so a text taller than the other 60%
    /// has less room to land in than one drag moves and can be stepped over every time
    private func tallerThanItsScrollArea(_ query: XCUIElementQuery) -> XCUIElement? {
        guard query.firstMatch.waitForExistence(timeout: 15) else { return nil }

        return query.allElementsBoundByIndex.first { $0.frame.height > readableArea(of: $0).height * 0.6 }
    }

    /// Scrolls a text too tall to show whole until its first line shows, then its last, as a user
    /// reading it through would; across the screen it must still fit the window
    private func assertBothEndsFit(_ element: XCUIElement, _ name: String) {
        let window = app.windows.firstMatch.frame
        XCTAssertTrue(
            element.frame.minX >= window.minX && element.frame.maxX <= window.maxX,
            "\(name) runs off the side of the screen at \(element.frame)"
        )

        // each end's line runs into the text from that edge, so the edge needs a line's room
        // inside the readable area on that side
        let line = Self.endLineHeight
        let ends: [(edge: (CGRect) -> CGFloat, room: (CGRect) -> ClosedRange<CGFloat>)] = [
            (\.minY, { $0.minY ... max($0.minY, $0.maxY - line) }),
            (\.maxY, { min($0.minY + line, $0.maxY) ... $0.maxY }),
        ]

        for (edge, room) in ends {
            for _ in 0 ..< 8 {
                let y = edge(element.frame)
                let fits = room(readableArea(of: element))
                if fits.contains(y) { break }

                drag(scrollContainers(holding: element).last ?? visibleScrollView() ?? app.windows.firstMatch, revealingContentBelow: y > fits.upperBound)
            }

            let y = edge(element.frame)
            let fits = room(readableArea(of: element))
            XCTAssertTrue(fits.contains(y), "\(name): its end at \(y) never scrolled into \(fits)")
        }
    }

    /// The part of the element's visible area below the top navigation bar, where content
    /// scrolled beneath the bar's translucent material no longer counts as shown
    private func readableArea(of element: XCUIElement) -> CGRect {
        let window = app.windows.firstMatch.frame
        let visible = visibleArea(of: element)
        let barBottom = app.navigationBars.allElementsBoundByIndex
            .map(\.frame)
            .filter { $0.minY > 0 && $0.maxY < window.midY }
            .map(\.maxY)
            .max() ?? visible.minY

        let top = min(max(visible.minY, barBottom), visible.maxY)
        return CGRect(x: visible.minX, y: top, width: visible.width, height: visible.maxY - top)
    }

    /// Scrolls down until a match exists, for a lazy grid or stack that builds only the cells near
    /// the screen, so `assertReachable` has one to scroll to
    func scrollDown(untilExists query: XCUIElementQuery) {
        for _ in 0 ..< 8 where !query.firstMatch.exists {
            scroll(revealingContentBelow: true)
        }
    }

    /// Drags the screen's scroll view by part of its height, as a user would
    func scroll(revealingContentBelow: Bool) {
        drag(visibleScrollView() ?? app.windows.firstMatch, revealingContentBelow: revealingContentBelow)
    }

    @discardableResult
    func reachAndTap(_ query: XCUIElementQuery, _ name: String) -> XCUIElement {
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
    func assertReachable(_ query: XCUIElementQuery, _ name: String, needsHit: Bool) -> XCUIElement {
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
    func visibleArea(of element: XCUIElement) -> CGRect {
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
}
