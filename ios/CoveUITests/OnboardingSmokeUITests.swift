import XCTest

final class OnboardingSmokeUITests: XCTestCase {
    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
    }

    func testWelcomeScreenShowsFirstLaunchEntryPoint() {
        app.launch()

        XCTAssertTrue(app.staticTexts["Welcome to Cove"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Get Started"].exists)
    }

    func testCanReachExistingWalletStorageChoices() {
        app.launch()

        app.buttons["Get Started"].tap()
        button(startingWith: "Yes, I have Bitcoin").tap()
        button(startingWith: "Use another wallet").tap()

        XCTAssertTrue(app.staticTexts["How do you store your Bitcoin?"].exists)
        XCTAssertTrue(button(startingWith: "On an exchange").exists)
        XCTAssertTrue(button(startingWith: "Hardware wallet").exists)
        XCTAssertTrue(button(startingWith: "Software wallet").exists)
    }

    private func button(startingWith labelPrefix: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }
}
