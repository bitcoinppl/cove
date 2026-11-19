import XCTest

/// Denies camera access on the prompt the first scan raises, and checks the app closes the
/// scanner, explains why, and is usable again after a trip to Settings
///
/// The prompt shows while the scanner's sheet slides in and makes the app inactive, which puts up
/// the privacy cover. The answer comes back on a queue of AVFoundation's own, and the scanner used
/// to hand the denial to the QR sheet there, which closed the sheet and changed app state off the
/// main thread just as the app came back and took the cover down. The simulator's scanner skips
/// the camera, so the debug camera prompt seam (`COVE_UI_TEST_CAMERA_PROMPT`) makes it ask the
/// way a phone's does. Needs the fake node, which `xtask ios-ui` (`just ios-ui-background`) starts
/// for every run
final class QrScanCameraPermissionFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    /// About as long as a person takes to read and answer the camera prompt
    private static let timeToAnswer: TimeInterval = 1.5

    private var app: XCUIApplication!
    private var springboard: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        app.launchEnvironment["COVE_UI_TEST_CAMERA_PROMPT"] = "1"
    }

    /// Don't Allow on the first scan's prompt, from Add Wallet's three-dot menu, closes the
    /// scanner and explains the camera is needed, and Add Wallet takes taps once the app is back
    /// from the Settings the explanation opens
    func testDenyingCameraAccessAsTheScannerOpensLeavesAddWalletUsable() {
        openAddWallet()
        openQrScannerFromMenu()
        denyCameraAccess()

        let explanation = app.alerts["Camera Access is Required"]
        XCTAssertTrue(explanation.waitForExistence(timeout: 15), "the app never explained the camera is needed")

        // the alert's only button opens Settings, so come back to the app the way a person would
        explanation.buttons["OK"].tap()
        let settings = XCUIApplication(bundleIdentifier: "com.apple.Preferences")
        XCTAssertTrue(settings.wait(for: .runningForeground, timeout: 10), "OK never opened Settings")
        app.activate()
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "the app never came back")

        let menu = app.buttons["More wallet import options"]
        let usable = NSPredicate { _, _ in menu.exists && menu.isHittable }
        let expectation = XCTNSPredicateExpectation(predicate: usable, object: nil)
        let result = XCTWaiter.wait(for: [expectation], timeout: 20)
        XCTAssertEqual(result, .completed, "something stayed up over Add Wallet")

        // a button XCUITest calls hittable can still sit under a layer that takes the tap, so
        // only the menu opening shows Add Wallet takes taps
        app.tapButton(named: "More wallet import options", until: app.buttons["QR"])
    }

    // MARK: - Steps

    /// Imports the test wallet, so Add Wallet is in the sidebar, then opens Add Wallet
    private func openAddWallet() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()

        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Add Wallet", until: app.buttons["More wallet import options"])
    }

    /// Opens the three-dot menu on Add Wallet and picks QR, without waiting for the scanner
    private func openQrScannerFromMenu() {
        app.tapButton(named: "More wallet import options", until: app.buttons["QR"])
        app.buttons["QR"].tap()
    }

    /// Waits for the system camera prompt and taps Don't Allow
    private func denyCameraAccess() {
        let prompt = springboard.alerts.firstMatch
        XCTAssertTrue(prompt.waitForExistence(timeout: 10), "the camera prompt never showed")
        Thread.sleep(forTimeInterval: Self.timeToAnswer)

        // the system writes Don't Allow with a curly apostrophe
        prompt.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Don")).firstMatch.tap()
        XCTAssertTrue(prompt.waitForNonExistence(timeout: 10), "the camera prompt never closed")
    }
}
