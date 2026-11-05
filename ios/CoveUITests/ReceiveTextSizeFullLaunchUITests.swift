import XCTest

/// Opens Receive at the largest accessibility text size and at the default size, and checks that
/// both offer the same controls, each reachable inside the window and enabled, and that the
/// largest size really took effect
///
/// One test sets the text size with the `-UIPreferredContentSizeCategoryName` launch argument,
/// which sets it for that launch only, so the default-size launch is the phone's text size put
/// back. The other changes the simulator's own text size through the hardware signer host while
/// Receive stays open, which the app gets as a live change, as when a person moves the slider in
/// Settings. Needs the fake node and the host, which `xtask ios-ui` (`just ios-ui-background`)
/// starts for every run
final class ReceiveTextSizeFullLaunchUITests: XCTestCase {
    private static let esploraUrl = "http://127.0.0.1:39001"
    private static let largestContentSize = "UICTContentSizeCategoryAccessibilityXXXL"

    /// Every control the Receive sheet offers once its address has loaded
    private static let receiveControls = ["Done", "Copy Address", "Create New Address"]

    /// How much taller Copy Address must be at the largest size, so a launch that ignored the text
    /// size argument fails. Its headline text grows from 17pt to 53pt while the padding stays the
    /// same. A view that capped its text below the largest size would still pass, which is right:
    /// the capped layout is what someone using the largest size sees
    private static let minimumGrowth: CGFloat = 1.3

    private var app: XCUIApplication!
    private let signer = HardwareSignerControl()

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
        app.openReceive()
        let largest = reachableReceiveControls()
        app.terminate()

        app.launchArguments = []
        app.launch(expecting: app.element("selectedWallet.balance"))
        app.openReceive()
        let normal = reachableReceiveControls()

        // equal controls prove nothing if the launch argument was ignored and both launches used the same size
        XCTAssertGreaterThan(normal.copyAddressHeight, 0, "Copy Address had no height at the default text size")
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

    /// Opens Receive at the largest text size, then sets the simulator back to the default size while
    /// the app keeps running and Receive stays open, and checks that the sheet lays out again at the
    /// new size with every control usable and the same address
    ///
    /// The launch argument the other test uses pins the size for the whole launch, so this one sets
    /// the simulator's own size through the host instead, both before the largest-size launch and
    /// for the live change, and puts back the size the simulator had once the test ends, even when
    /// it fails
    func testReceiveLaysOutAgainWhenTextSizeChangesWhileOpen() throws {
        try signer.requireHealthy()
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.terminate()

        try setSimulatorContentSize(.accessibilityExtraExtraExtraLarge)
        app.launch(expecting: app.element("selectedWallet.balance"))
        app.openReceive()
        let largest = reachableReceiveControls()
        let largestAddress = receiveAddress()
        XCTAssertFalse(largestAddress.isEmpty, "Receive shows no address at the largest text size")

        try setSimulatorContentSize(.large)
        waitForCopyAddressToShrink(from: largest.copyAddressHeight)
        let restored = reachableReceiveControls()

        XCTAssertEqual(restored.controls, Self.receiveControls, "controls missing from Receive after the text size went back to the default")
        XCTAssertEqual(largest.controls, restored.controls, "Receive offered different controls at the largest text size")
        XCTAssertEqual(receiveAddress(), largestAddress, "Receive shows another address after the text size changed")
    }

    /// Lists the controls the open Receive sheet offers that can be used, scrolling the sheet
    /// toward any that are off screen, as a user would, along with the height Copy Address renders
    /// at once usable
    private func reachableReceiveControls() -> (controls: [String], copyAddressHeight: CGFloat) {
        let copyAddress = button("Copy Address")
        XCTAssertTrue(copyAddress.waitForExistence(timeout: 10), "Receive has no Copy Address button")

        // measured only once the button is usable, since a frame read mid-layout can be empty
        var copyAddressHeight: CGFloat = 0
        var found = Set<String>()
        for _ in 0 ..< 6 {
            found.formUnion(Self.receiveControls.filter { isUsable(button($0)) })
            if found.contains("Copy Address"), copyAddressHeight == 0 {
                copyAddressHeight = copyAddress.frame.height
            }

            if found.count == Self.receiveControls.count { break }

            scrollReceiveSheet()
        }

        return (Self.receiveControls.filter(found.contains), copyAddressHeight)
    }

    /// Whether a teardown block already puts back the size the simulator had before this test
    private var restoresContentSize = false

    /// Sets the simulator's text size while the app runs, and the first time puts back the size the
    /// simulator had once the test ends, whether it passes or fails
    private func setSimulatorContentSize(_ size: HardwareSignerControl.ContentSize) throws {
        // registered before the first change, so a change whose answer never arrives is still undone
        if !restoresContentSize {
            let original = try signer.simulatorContentSize()
            let signer = signer
            addTeardownBlock {
                try signer.setSimulatorContentSize(original)
            }

            restoresContentSize = true
        }

        try signer.setSimulatorContentSize(size)
    }

    /// Waits for Copy Address to lay out again at the default size, which proves the sheet took the
    /// live change rather than keeping the layout it had at the largest size
    private func waitForCopyAddressToShrink(from largestHeight: CGFloat) {
        let copyAddress = button("Copy Address")
        let deadline = Date().addingTimeInterval(10)
        var height: CGFloat = 0

        // the sheet can drop and rebuild its controls while it lays out again, so a missing or empty
        // frame only means it isn't done yet
        repeat {
            height = copyAddress.exists ? copyAddress.frame.height : 0
            if height > 0, height * Self.minimumGrowth <= largestHeight { return }

            Thread.sleep(forTimeInterval: 0.25)
        } while Date() < deadline

        XCTAssertGreaterThan(height, 0, "Copy Address is gone after the text size changed")
        XCTAssertLessThanOrEqual(
            height * Self.minimumGrowth,
            largestHeight,
            "Copy Address is still \(height)pt tall after the text size went back to the default, \(largestHeight)pt at the largest, so Receive didn't lay out again"
        )
    }

    /// The address the open Receive sheet shows, spaced into groups as drawn
    private func receiveAddress() -> String {
        let address = app.staticTexts["receive.address"]
        XCTAssertTrue(address.waitForExistence(timeout: 10), "Receive shows no address")
        return address.label
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
