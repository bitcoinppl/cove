import XCTest

/// Unlocks the real app with the simulator's Face ID after a cold start, failing and cancelling
/// the system prompt first
///
/// The UI test host enrolls a face and answers the Face ID prompt with a matching or non-matching
/// face (docs/testing.md). Needs the host, which `xtask ios-ui` (`just ios-ui-background`) starts
/// for every run, and a freshly erased simulator, which xtask also provides
final class BiometricLockFullLaunchUITests: XCTestCase {
    private static let pin = "135791"

    /// The apps the Face ID prompt and its permission alert may belong to; which one draws them
    /// isn't documented and has moved between iOS versions
    private static let systemUiBundles = ["com.apple.springboard", "com.apple.CoreAuthUI"]

    /// The Cancel of the Face Not Recognized alert, which springboard draws on iOS 27
    private static let faceIdCancel = "com.apple.localauthentication.ax.authentication.button.cancel"

    private var app: XCUIApplication!
    private var biometrics: SimulatorBiometrics!
    private var layout: ScreenReachability!

    override func setUpWithError() throws {
        try super.setUpWithError()
        continueAfterFailure = false

        app = XCUIApplication()
        biometrics = SimulatorBiometrics()
        layout = ScreenReachability(app: app)
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = UITestHostPorts.esploraUrl

        try HardwareSignerControl().requireHealthy()
        try biometrics.enroll()
    }

    override func tearDownWithError() throws {
        // runs after a failure too, so a later test on this simulator never starts with a face
        // enrolled
        try biometrics?.unenroll()
        try super.tearDownWithError()
    }

    /// A failed or cancelled Face ID prompt on a cold start leaves the wallet locked, and Tap to
    /// Unlock asks again and unlocks with the enrolled face
    func testCancelledFaceIdKeepsTheAppLockedUntilAMatch() throws {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        try enablePinAndFaceId()

        // a cold start locks the app and asks for Face ID straight away
        app.terminate()
        app.launch()
        let tapToUnlock = app.staticTexts["Tap to Unlock"]
        let balance = app.element("selectedWallet.balance")
        XCTAssertTrue(tapToUnlock.waitForExistence(timeout: 30), "a cold start with Face ID on should lock the app")

        // read before the prompt is answered, so it shows what the lock screen hides on its own
        let readableUnderLock = balance.exists

        let cancel = try failFaceId()
        cancelFaceId(cancel)

        // gives an unlock that would follow the cancel time to happen before asserting it didn't
        _ = tapToUnlock.waitForNonExistence(timeout: 3)
        attachScreenshot("after Face ID was cancelled")
        XCTAssertTrue(tapToUnlock.exists && tapToUnlock.isHittable, "the lock screen should stay after Face ID was cancelled")
        XCTAssertFalse(balance.exists && balance.isHittable, "the wallet showed after Face ID was cancelled")
        XCTAssertFalse(readableUnderLock, "the wallet is readable through accessibility under the lock screen")

        // the app must have let go of the cancelled prompt, or this tap asks for nothing
        tapToUnlock.tap()
        try matchFaceId(until: tapToUnlock, matching: NSPredicate(format: "exists == false"))
        XCTAssertTrue(tapToUnlock.waitForNonExistence(timeout: 10), "the lock screen stayed after a matching face")
        XCTAssertTrue(balance.waitForExistence(timeout: 10), "the wallet should show once Face ID matched")
    }

    // MARK: - Steps

    /// Sets a PIN, then turns on Face ID, which asks for a face before it saves
    private func enablePinAndFaceId() throws {
        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Settings")
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))

        // each row is a switch wrapping the control itself, which is what takes the tap
        let pinToggle = layout.assertReachable(app.switches.matching(identifier: "Enable PIN"), "settings: Enable PIN", needsHit: true)
        pinToggle.switches.firstMatch.tap()
        XCTAssertTrue(app.staticTexts["Enter New PIN"].waitForExistence(timeout: 10))
        enterPin()
        XCTAssertTrue(app.staticTexts["Confirm New PIN"].waitForExistence(timeout: 5))
        enterPin()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))
        XCTAssertEqual(pinToggle.value as? String, "1", "the PIN was not saved")

        // Settings offers Face ID only once the simulator has a face enrolled
        let faceIdToggle = layout.assertReachable(app.switches.matching(identifier: "Enable FaceID"), "settings: Enable FaceID", needsHit: true)
        faceIdToggle.switches.firstMatch.tap()
        allowFaceIdIfAsked()

        let saved = NSPredicate(format: "value == '1'")
        try matchFaceId(until: faceIdToggle, matching: saved)
        XCTAssertEqual(faceIdToggle.value as? String, "1", "Face ID was not turned on")
    }

    /// Answers the Face ID prompt with a face that isn't enrolled, and returns the prompt's Cancel
    private func failFaceId() throws -> XCUIElement {
        let deadline = Date().addingTimeInterval(30)

        // the prompt takes a moment to start scanning, and a face shown before then is ignored, so
        // show it again until the prompt reports the failure
        while Date() < deadline {
            try biometrics.nomatch()

            if let cancel = systemButton(Self.faceIdCancel, timeout: 3) { return cancel }
        }

        attachSystemUi("face id prompt after nomatch")
        throw ControlRequestError(description: "the Face ID prompt never offered Cancel after a face that isn't enrolled")
    }

    /// Shows the enrolled face to the Face ID prompt until `target` matches, since a face shown
    /// before the prompt starts scanning is ignored
    private func matchFaceId(until target: XCUIElement, matching predicate: NSPredicate = NSPredicate(format: "exists == true")) throws {
        let deadline = Date().addingTimeInterval(30)

        while Date() < deadline {
            try biometrics.match()

            let met = XCTNSPredicateExpectation(predicate: predicate, object: target)
            if XCTWaiter.wait(for: [met], timeout: 3) == .completed { return }
        }

        attachSystemUi("face id prompt after match")
        XCTFail("a matching face never got past the Face ID prompt")
    }

    /// Taps the Face ID alert's Cancel until the alert goes, since a tap while the alert is still
    /// animating in does nothing
    private func cancelFaceId(_ cancel: XCUIElement) {
        for _ in 0 ..< 3 {
            if cancel.exists, cancel.isHittable { cancel.tap() }
            if cancel.waitForNonExistence(timeout: 3) { return }
        }

        XCTFail("Cancel should close the Face ID prompt")
    }

    /// The first Face ID request asks whether Cove may use Face ID
    private func allowFaceIdIfAsked() {
        if let allow = systemButton("OK", timeout: 5) ?? systemButton("Allow", timeout: 1) {
            allow.tap()
        }
    }

    // MARK: - Helpers

    /// A button of the system UI drawn over the app, wherever iOS draws it, by label or identifier
    private func systemButton(_ name: String, timeout: TimeInterval) -> XCUIElement? {
        // the Face ID alert's buttons each nest a copy of themselves, so take the outer one
        let candidates = Self.systemUiBundles.map { XCUIApplication(bundleIdentifier: $0).buttons[name].firstMatch }
        let deadline = Date().addingTimeInterval(timeout)

        repeat {
            if let found = candidates.first(where: \.exists) { return found }
            Thread.sleep(forTimeInterval: 0.25)
        } while Date() < deadline

        return nil
    }

    private func attachScreenshot(_ name: String) {
        let attachment = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    /// Attaches what the system UI apps show, so a failed run says where the prompt lived
    private func attachSystemUi(_ name: String) {
        let targets = [("Cove", app!)] + Self.systemUiBundles.map { ($0, XCUIApplication(bundleIdentifier: $0)) }

        for (label, target) in targets {
            let attachment = XCTAttachment(string: target.debugDescription)
            attachment.name = "\(name): \(label)"
            attachment.lifetime = .keepAlways
            add(attachment)
        }
    }

    private func enterPin() {
        for digit in Self.pin {
            let key = app.buttons[String(digit)]
            XCTAssertTrue(key.waitForExistence(timeout: 5), "number pad key \(digit) is missing")
            key.tap()
        }
    }
}
