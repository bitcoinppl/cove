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
        assertRenameKeepsKeyboardAndSaves(openRename: openRenameFromTitleMenu)
    }

    func testRenameFromWalletSettingsKeepsTheKeyboard() {
        launchWallet()

        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Wallet Settings")

        assertRenameKeepsKeyboardAndSaves(openRename: openRenameFromWalletSettings)
    }

    // MARK: - Steps

    /// Imports the test wallet and lands on its wallet screen
    private func launchWallet() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
    }

    private func openRenameFromTitleMenu() {
        app.element("selectedWallet.title").press(forDuration: 1.0)
        app.tapButton(named: "Change Name")
    }

    private func openRenameFromWalletSettings() {
        let nameRow = app.element("walletSettings.name")
        XCTAssertTrue(nameRow.waitForExistence(timeout: 10), "wallet settings never showed the name row")
        nameRow.tap()
    }

    /// Renames the wallet through the rename screen, then reopens it to check the name was saved
    private func assertRenameKeepsKeyboardAndSaves(openRename: () -> Void) {
        openRename()
        let renamed = assertNameFieldKeepsKeyboard()

        // the field keeps its own draft, so only a freshly opened screen shows the saved name
        let field = app.textFields["walletSettings.changeName.field"]
        app.navigationBars.buttons.element(boundBy: 0).tap()
        XCTAssertTrue(field.waitForNonExistence(timeout: 10), "the rename screen never closed")

        openRename()
        XCTAssertTrue(field.waitForExistence(timeout: 10), "rename screen never reopened")
        waitUntil(
            field,
            matches: NSPredicate(format: "value == %@", renamed),
            timeout: 10,
            "the reopened rename screen didn't show the typed name, so it was never saved"
        )
    }

    /// Returns the name the field holds after typing into it
    private func assertNameFieldKeepsKeyboard() -> String {
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
        let renamed = "\(originalName) Renamed"
        app.typeText(" Renamed")
        waitUntil(
            field,
            matches: NSPredicate(format: "value == %@", renamed),
            timeout: 5,
            "typed text never reached the name field"
        )

        return renamed
    }
}
