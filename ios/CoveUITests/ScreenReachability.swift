import XCTest

/// Scrolls a screen as a user would and checks that its text and actions fit inside the window
///
/// The checks catch text and actions pushed off screen, cut off by their scroll view or out of
/// reach. XCUITest reports a text's whole label even when it draws truncated, so wherever a text is
/// scrolled into view they also read the screenshot with Vision and fail on an ellipsis
struct ScreenReachability {
    /// Inside the screens' 16 to 20 point side padding, so a drag there lands on no field
    private static let dragMargin: CGFloat = 8

    /// Taller than the keyboard's suggestion bar and shorter than any screen's scroll area
    private static let rowHeight: CGFloat = 100

    let app: XCUIApplication

    /// Every text and action must lie wholly inside the window once scrolled to, the actions must
    /// take a tap there, and no text on screen may draw truncated wherever one of them is scrolled
    /// into view
    ///
    /// Vision must also read each text and each button's label where it is drawn. `controls` are
    /// actions without a drawn label, such as icon-only buttons and fields, checked for reach only
    func assertScreenFits(_ screen: String, texts: [String], buttons: [XCUIElementQuery], controls: [XCUIElementQuery] = []) {
        for text in texts {
            let element = assertReachable(app.staticTexts.matching(NSPredicate(format: "label == %@", text)), "\(screen): '\(text)'", needsHit: false)
            assertDrawn(text, in: element, "\(screen): '\(text)'")
            assertNoTruncatedText("\(screen) at '\(text)'")
        }

        for button in buttons {
            let name = button.firstMatch.description
            let element = assertReachable(button, "\(screen): \(name)", needsHit: true)
            assertDrawn(element.label, in: element, "\(screen): \(name)")
            assertNoTruncatedText("\(screen) at \(name)")
        }

        for control in controls {
            let name = control.firstMatch.description
            assertReachable(control, "\(screen): \(name)", needsHit: true)
            assertNoTruncatedText("\(screen) at \(name)")
        }
    }

    /// Fails when Vision reads an ellipsis on screen, which SwiftUI draws in place of the end of a
    /// text it truncates
    func assertNoTruncatedText(_ screen: String) {
        let truncated = app.visibleText.filter { $0.contains("…") || $0.contains("...") }
        XCTAssertTrue(truncated.isEmpty, "\(screen): text draws truncated: \(truncated)")
    }

    /// Fails unless Vision reads the start and the end of the text where it is drawn, so a text
    /// clipped without an ellipsis fails, and the ellipsis check above is reading the screen it is
    /// meant to and not, say, one still sliding in
    ///
    /// Eight characters at each end, allowing one misread, since across a whole key or address a
    /// misread is likelier than missing text, and clipping cuts off an end
    func assertDrawn(_ text: String, in element: XCUIElement, _ name: String) {
        let compact = text.filter { !$0.isWhitespace }
        for part in [String(compact.prefix(8)), String(compact.suffix(8))] {
            XCTAssertTrue(element.waitForVisibleText(containing: part, misreads: 1, timeout: 3), "\(name): Vision never read '\(part)' where it is drawn")
        }
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
}
