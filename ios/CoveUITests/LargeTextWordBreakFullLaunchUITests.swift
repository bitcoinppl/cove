import XCTest

/// Opens the wallet screen, Add New Wallet, the xpub export, Settings and the PIN pad at the
/// largest accessibility text size, and checks that each label draws whole, with no word split
/// across lines by an added hyphen
///
/// iOS hyphenates a word too long for its line at accessibility sizes, so two buttons side by side
/// drew "Re-" over "ceive" and "Hard-" over "ware". The truncation probe reports `hyphenated` for
/// such a text. Needs the fake node, which `xtask ios-ui` (`just ios-ui-background`) starts for
/// every run
final class LargeTextWordBreakFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl
    private static let largestContentSize = "UICTContentSizeCategoryAccessibilityXXXL"

    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
    }

    func testWalletScreenAndAddNewWalletDrawWholeAtLargestTextSize() {
        launchWalletAtLargestTextSize()
        let screen = ScreenReachability(app: app)

        screen.assertScreenFits("wallet", texts: [], buttons: [button("Send"), button("Receive")])

        // XCUITest reports these two buttons unhittable under a layer Add New Wallet draws over
        // them, though a tap at their center lands, so their labels are checked as texts
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Add Wallet")
        screen.assertScreenFits("Add New Wallet", texts: ["Hardware Wallet", "On This Device"], buttons: [])
    }

    /// The wallet's xpub leaves only as a QR code, a file or a copy, so the sheet's title and
    /// subtitle are its only texts
    func testXpubExportDrawsWholeAtLargestTextSize() {
        launchWalletAtLargestTextSize()

        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Export Xpub")
        app.tapButton(named: "QR Code")
        ScreenReachability(app: app).assertScreenFits(
            "xpub export",
            texts: ["Export Xpub"],
            descriptions: ["Public descriptor for"],
            buttons: []
        )

        // the probe can't see a button drawn over a text, and the copy button once sat over the
        // title; its hidden twin that centers the title must stay out of the accessibility tree
        let copy = app.buttons.matching(NSPredicate(format: "label == %@", "Copy"))
        XCTAssertEqual(copy.count, 1, "xpub export: expected one Copy button")

        let title = app.staticTexts["Export Xpub"].frame
        XCTAssertFalse(
            title.intersects(copy.firstMatch.frame),
            "xpub export: Copy at \(copy.firstMatch.frame) overlaps the title at \(title)"
        )
    }

    /// Side by side with its icon, a Settings title such as "Appearance" split with a hyphen, and
    /// the PIN pad's Cancel split in its third of the width
    func testSettingsAndPinEntryDrawWholeAtLargestTextSize() {
        launchWalletAtLargestTextSize()

        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Settings")
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))

        let screen = ScreenReachability(app: app)
        screen.assertScreenFits(
            "settings",
            texts: ["Network", "Appearance", "Node", "Block Explorer", "Currency"],
            buttons: []
        )

        // the list builds rows only near the screen, and at this size the security rows start
        // far below it; the row is a switch wrapping the control itself, which takes the tap
        screen.scrollDown(untilExists: app.switches.matching(identifier: "Enable PIN"))
        let pinToggle = screen.assertReachable(app.switches.matching(identifier: "Enable PIN"), "settings: Enable PIN", needsHit: true)
        pinToggle.switches.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Enter New PIN"].waitForExistence(timeout: 10))
        screen.assertScreenFits(
            "new PIN",
            texts: ["Enter New PIN"],
            buttons: [app.buttons.matching(NSPredicate(format: "label == %@", "Cancel"))]
        )
    }

    /// Imports the test wallet, then relaunches onto it at the largest text size with the
    /// truncation probe on
    private func launchWalletAtLargestTextSize() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.terminate()

        let (probeKey, probeValue) = ScreenReachability.truncationProbeEnvironment
        app.launchEnvironment[probeKey] = probeValue
        app.launchArguments = ["-UIPreferredContentSizeCategoryName", Self.largestContentSize]
        app.launch(expecting: app.element("selectedWallet.balance"))
    }

    /// Buttons whose label starts with the title, since a label can also hold its icon's name
    private func button(_ title: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", title))
    }
}
