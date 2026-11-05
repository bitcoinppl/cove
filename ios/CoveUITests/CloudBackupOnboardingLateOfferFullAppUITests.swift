import XCTest

/// Startup cloud discovery that answers only after the person has moved on to importing a wallet,
/// so OnboardingManager reconciles the late backup into the import screen's offer
final class CloudBackupOnboardingLateOfferFullAppUITests: XCTestCase {
    private var app: XCUIApplication!
    private var robot: CloudBackupHostRobot!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
        robot = CloudBackupHostRobot(app: app)
    }

    override func tearDown() {
        CloudBackupHostControl.tearDown(self, app: app)
        super.tearDown()
    }

    func testLateDiscoveryOffersRestoreDuringSoftwareImport() throws {
        let control = try launchWithDiscoveryHeld()

        chooseStorage(startingWith: "Software wallet")
        XCTAssertTrue(app.staticTexts["Import your software wallet"].waitForExistence(timeout: 10))
        XCTAssertTrue(robot.button(startingWith: "Enter recovery words").exists)
        XCTAssertFalse(app.alerts["Cove backup found"].exists)

        control.release("discovery")

        let offer = lateOffer()
        offer.buttons["Continue setup"].tap()
        XCTAssertTrue(offer.waitForNonExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Import your software wallet"].exists)
        XCTAssertTrue(robot.button(startingWith: "Enter recovery words").exists)
        XCTAssertTrue(robot.button(startingWith: "Scan QR code").exists)
    }

    func testLateDiscoveryOffersRestoreDuringHardwareImport() throws {
        let control = try launchWithDiscoveryHeld()

        chooseStorage(startingWith: "Hardware wallet")
        XCTAssertTrue(app.staticTexts["Import your hardware wallet"].waitForExistence(timeout: 10))
        XCTAssertTrue(robot.button(startingWith: "Scan export QR").exists)
        XCTAssertFalse(app.alerts["Cove backup found"].exists)

        control.release("discovery")

        let offer = lateOffer()
        offer.buttons["Restore from Cove backup"].tap()
        XCTAssertTrue(app.staticTexts["iCloud Backup Found"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["Restore with Passkey"].waitForExistence(timeout: 10))
    }

    /// Launches with the startup namespace listing held, and checks the app is still waiting on it
    private func launchWithDiscoveryHeld() throws -> CloudBackupHostControl {
        let control = try CloudBackupHostControl(script: .init(
            cloud: .fixture,
            namespaceResults: [.backupFound],
            holds: [.init(id: "discovery", point: .namespaceList)]
        ))
        robot.launch(control: control)
        control.waitForHold("discovery")
        XCTAssertTrue(app.staticTexts["Welcome to Cove"].exists)

        return control
    }

    private func chooseStorage(startingWith title: String) {
        let getStarted = app.buttons["Get Started"]
        robot.scrollToHittable(getStarted)
        getStarted.tap()

        if app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 2) {
            robot.acceptTerms()
        }

        let existingUser = robot.button(startingWith: "Yes, I have Bitcoin")
        robot.scrollToHittable(existingUser)
        existingUser.tap()
        XCTAssertTrue(app.staticTexts["How do you store your Bitcoin?"].waitForExistence(timeout: 10))

        let storage = robot.button(startingWith: title)
        robot.scrollToHittable(storage)
        storage.tap()
    }

    private func lateOffer() -> XCUIElement {
        let offer = app.alerts["Cove backup found"]
        XCTAssertTrue(offer.waitForExistence(timeout: 15), "the late cloud restore offer never appeared")
        XCTAssertTrue(offer.staticTexts["We found a cloud backup for this account."].exists)
        XCTAssertTrue(offer.buttons["Restore from Cove backup"].exists)
        XCTAssertTrue(offer.buttons["Continue setup"].exists)

        return offer
    }
}
