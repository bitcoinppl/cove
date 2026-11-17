import XCTest

/// Opens the rename screen and checks its name field keeps the keyboard
///
/// The rename screen focuses its name field as it appears, and the keyboard used to rise and then
/// vanish on its own as the push finished, with nothing typed. Fast typing also used to lose
/// letters while the saved name caught up. Needs the fake node, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run
final class RenameWalletFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
    }

    func testRenameFromTitleMenuKeepsTheKeyboard() {
        launchWallet()

        app.element("selectedWallet.title").press(forDuration: 1.0)
        app.tapButton(named: "Change Name")

        assertNameFieldKeepsKeyboard()
    }

    func testRenameFromWalletSettingsKeepsTheKeyboard() {
        launchWallet()

        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Wallet Settings")
        let nameRow = app.element("walletSettings.name")
        XCTAssertTrue(nameRow.waitForExistence(timeout: 10), "wallet settings never showed the name row")
        nameRow.tap()

        assertNameFieldKeepsKeyboard()
    }

    // MARK: - Steps

    /// Imports the test wallet and lands on its wallet screen
    private func launchWallet() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
    }

    private func assertNameFieldKeepsKeyboard() {
        let field = app.textFields["walletSettings.changeName.field"]
        XCTAssertTrue(field.waitForExistence(timeout: 10), "rename screen never opened")
        let originalName = field.value as? String ?? ""
        XCTAssertTrue(field.wait(for: \.hasKeyboardFocus, toEqual: true, timeout: 5), "rename screen never focused the name field")

        // the keyboard used to vanish within a second of rising, so the focus must outlast that
        XCTAssertFalse(
            field.wait(for: \.hasKeyboardFocus, toEqual: false, timeout: 3),
            "the name field lost the keyboard on its own after the rename screen opened"
        )

        // typing without tapping the field proves typed text still reaches it
        app.typeText(" Renamed")
        waitUntil(
            field,
            matches: NSPredicate(format: "value == %@", "\(originalName) Renamed"),
            timeout: 5,
            "typed text never reached the name field"
        )
    }
}
