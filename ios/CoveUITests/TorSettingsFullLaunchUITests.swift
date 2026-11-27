import XCTest

/// The Tor screen's proxy form: Rust refuses an address that isn't host:port, which shows under
/// the field, saves a valid one, and turning Tor off goes back to direct connections
///
/// Nothing listens on the test proxy's port, so the test checks only what the screen shows, which
/// doesn't need the proxy to carry traffic. Needs the fake node, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run
final class TorSettingsFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
    }

    func testProxyFormRefusesAnInvalidAddressThenSavesAndTurnsOff() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        openTorSettings()

        let status = app.staticTexts["tor.status"]
        XCTAssertTrue(status.waitForExistence(timeout: 10), "the Tor screen never showed its status")
        waitForLabel(status, equalTo: "Off", timeout: 5)
        XCTAssertTrue(app.buttons["tor.mode.off"].isSelected, "a fresh install didn't start with Tor off")

        // picking the proxy only opens its form, so nothing is saved yet
        app.tapButton(named: "tor.mode.proxy")
        let field = app.textFields["tor.proxy.address"]
        XCTAssertTrue(field.waitForExistence(timeout: 5), "picking Tor proxy never showed the proxy form")
        waitForLabel(status, equalTo: "Off", timeout: 5)

        replaceText(in: field, with: "not a proxy")
        app.tapButton(named: "tor.proxy.use")
        let error = app.staticTexts["tor.proxy.error"]
        XCTAssertTrue(error.waitForExistence(timeout: 5), "an invalid proxy address showed no error")
        XCTAssertFalse(error.label.isEmpty, "the proxy error has no text")
        waitForLabel(status, equalTo: "Off", timeout: 5)

        // editing the address clears the old error
        replaceText(in: field, with: "127.0.0.1:9")
        XCTAssertTrue(error.waitForNonExistence(timeout: 5), "the proxy error stayed after the address changed")
        app.tapButton(named: "tor.proxy.use")
        waitForLabel(status, equalTo: "Using proxy", timeout: 15)
        XCTAssertFalse(error.exists, "a valid proxy address showed an error")

        app.tapButton(named: "tor.mode.off")
        waitForLabel(status, equalTo: "Off", timeout: 15)
        XCTAssertTrue(field.waitForNonExistence(timeout: 5), "turning Tor off left the proxy form open")
    }

    // MARK: - Steps

    /// Opens Settings, checks its Tor row shows Tor off, and opens the Tor screen
    private func openTorSettings() {
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Settings")
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))

        let rowValue = app.staticTexts["settings.tor.value"]
        XCTAssertTrue(rowValue.waitForExistence(timeout: 10), "Settings has no Tor row value")
        waitForLabel(rowValue, equalTo: "Off", timeout: 5)

        // the row is a title with a tap gesture, not a button
        app.staticTexts["Tor"].tap()
        XCTAssertTrue(app.navigationBars["Tor"].waitForExistence(timeout: 10), "the Tor row never opened the Tor screen")
    }

    /// Clears the field and types the text, as a user editing the prefilled address would
    private func replaceText(in field: XCUIElement, with text: String) {
        XCTAssertTrue(field.waitForExistence(timeout: 5))

        // the address is right-aligned, so a tap at the trailing edge puts the caret after it and
        // the deletes clear all of it, where a tap in a focused field's middle would split it
        field.coordinate(withNormalizedOffset: CGVector(dx: 0.98, dy: 0.5)).tap()

        let current = field.value as? String ?? ""
        let deletes = String(repeating: XCUIKeyboardKey.delete.rawValue, count: current.count)
        field.typeTextOnceFocused(deletes + text)
        waitUntil(field, matches: NSPredicate(format: "value == %@", text), timeout: 5, "the proxy field didn't take the typed address")
    }
}
