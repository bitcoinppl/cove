import XCTest

/// Opens Receive at the largest accessibility text size and again at the default size, and checks
/// that both offer the same controls, each reachable inside the window and enabled, and that the
/// largest size really took effect
///
/// The text size comes from the `-UIPreferredContentSizeCategoryName` launch argument, which sets
/// it for that launch only, so the default-size launch is the phone's text size put back. Needs the
/// fake node, which `xtask ios-ui` (`just ios-ui-background`) starts for every run
final class ReceiveTextSizeFullLaunchUITests: XCTestCase {
    private static let esploraUrl = "http://127.0.0.1:39001"
    private static let largestContentSize = "UICTContentSizeCategoryAccessibilityXXXL"

    /// Every control the Receive sheet offers once its address has loaded
    private static let receiveControls = ["Done", "Copy Address", "Create New Address"]

    /// How much taller Copy Address must be at the largest size, where its headline text grows from
    /// 17pt to 53pt while the padding around it stays the same
    private static let minimumGrowth: CGFloat = 1.3

    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
    }

    func testReceiveControlsMatchAtLargestAndDefaultTextSize() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.terminate()

        app.launchArguments = ["-UIPreferredContentSizeCategoryName", Self.largestContentSize]
        app.launch(expecting: app.element("selectedWallet.balance"))
        let largest = reachableReceiveControls()
        app.terminate()

        app.launchArguments = []
        app.launch(expecting: app.element("selectedWallet.balance"))
        let normal = reachableReceiveControls()

        // equal controls prove nothing if the launch argument was ignored and both launches used the same size
        XCTAssertGreaterThanOrEqual(
            largest.copyAddressHeight,
            normal.copyAddressHeight * Self.minimumGrowth,
            "Copy Address is \(largest.copyAddressHeight)pt tall at the largest size and \(normal.copyAddressHeight)pt at the default, so the largest size didn't take effect"
        )
        XCTAssertEqual(normal.controls, Self.receiveControls, "controls missing from Receive at the default text size")
        XCTAssertEqual(
            largest.controls,
            normal.controls,
            "Receive at the largest text size offers different controls from the default size"
        )
    }

    /// Opens Receive and lists the controls that can be used, scrolling the sheet toward any that
    /// are off screen, as a user would, along with the height Copy Address renders at
    private func reachableReceiveControls() -> (controls: [String], copyAddressHeight: CGFloat) {
        app.openReceive()

        let copyAddress = button("Copy Address")
        XCTAssertTrue(copyAddress.waitForExistence(timeout: 10), "Receive has no Copy Address button")
        let copyAddressHeight = copyAddress.frame.height

        var found = Set<String>()
        for _ in 0 ..< 6 {
            found.formUnion(Self.receiveControls.filter { isUsable(button($0)) })
            if found.count == Self.receiveControls.count { break }

            scrollReceiveSheet()
        }

        return (Self.receiveControls.filter(found.contains), copyAddressHeight)
    }

    /// A control counts only when it lies wholly inside the window, takes a tap and is enabled
    private func isUsable(_ element: XCUIElement) -> Bool {
        guard element.exists, element.isEnabled else { return false }

        let window = app.windows.firstMatch.frame
        return !element.frame.isEmpty && window.contains(element.frame) && element.isHittable
    }

    /// Drags the sheet's content up slowly, so it scrolls without a fling carrying a control past
    /// the window
    private func scrollReceiveSheet() {
        let window = app.windows.firstMatch.frame
        let sheet = app.scrollViews.containing(.button, identifier: "Copy Address").allElementsBoundByIndex
            .min { $0.frame.height < $1.frame.height }
        let area = sheet.map { $0.frame.intersection(window) } ?? window

        let origin = app.windows.firstMatch.coordinate(withNormalizedOffset: .zero)
        let point = { (fraction: CGFloat) in
            origin.withOffset(CGVector(dx: area.midX, dy: area.minY + area.height * fraction))
        }

        point(0.7).press(forDuration: 0.05, thenDragTo: point(0.3), withVelocity: .slow, thenHoldForDuration: 0.1)
    }

    private func button(_ label: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label == %@", label)).firstMatch
    }
}
