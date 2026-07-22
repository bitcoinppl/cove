import XCTest

final class CloudBackupInventoryFullAppUITests: XCTestCase {
    private let firstRecordId = "24a462de41c1dcbfba825e87674d526b4fc0b7811fb0cee65232cb4f8209c11d"
    private let secondRecordId = "b1089e1017cd2b2a2e0085048b346f2e1c4a850a65b69a43b8d839cd60bd3f6b"
    private let thirdRecordId = "fa5eab18295a70b89491f347427099da2dd9d528e59d85b8934084fa01119914"

    private var app: XCUIApplication!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false
        app = XCUIApplication()
    }

    func testSCU02ProvisionalSnapshotAdvancesToAuthoritativeInventoryUnion() {
        launchAndOpenCloudBackupDetail(scenario: "SC-U02")

        XCTAssertTrue(inventoryChecking.waitForExistence(timeout: 10))
        XCTAssertTrue(walletRow(recordId: firstRecordId).waitForExistence(timeout: 10))
        XCTAssertFalse(walletRow(recordId: secondRecordId).exists)
        XCTAssertFalse(walletRow(recordId: thirdRecordId).exists)

        XCTAssertTrue(walletRow(recordId: secondRecordId).waitForExistence(timeout: 15))
        XCTAssertTrue(walletRow(recordId: thirdRecordId).exists)
        XCTAssertTrue(inventoryChecking.waitForNonExistence(timeout: 5))
        XCTAssertFalse(inventoryIncomplete.exists)
    }

    func testSCU03TimeoutRetainsKnownRowAndCheckAgainCompletesInventory() {
        launchAndOpenCloudBackupDetail(scenario: "SC-U03")

        let knownRow = walletRow(recordId: firstRecordId)
        XCTAssertTrue(inventoryChecking.waitForExistence(timeout: 10))
        XCTAssertTrue(knownRow.waitForExistence(timeout: 10))
        XCTAssertTrue(inventoryIncomplete.waitForExistence(timeout: 15))
        XCTAssertTrue(knownRow.exists)
        XCTAssertFalse(app.staticTexts["No iCloud backup files uploaded yet"].exists)
        XCTAssertFalse(walletRow(recordId: secondRecordId).exists)
        XCTAssertFalse(walletRow(recordId: thirdRecordId).exists)

        let checkAgain = app.buttons["cloudBackup.inventory.checkAgain"]
        XCTAssertTrue(checkAgain.waitForExistence(timeout: 5))
        XCTAssertTrue(checkAgain.isHittable)
        checkAgain.tap()

        XCTAssertTrue(walletRow(recordId: secondRecordId).waitForExistence(timeout: 20))
        XCTAssertTrue(walletRow(recordId: thirdRecordId).exists)
        XCTAssertTrue(knownRow.exists)
        XCTAssertTrue(inventoryIncomplete.waitForNonExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["No iCloud backup files uploaded yet"].exists)
    }

    func testMAN03NativePasskeyCreateAssertAndPrfSmoke() {
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_RESET"] = "1"
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_SCENARIO"] = "MAN-03"
        app.launch()

        XCTAssertTrue(app.staticTexts["Welcome to Cove"].waitForExistence(timeout: 10))
        app.buttons["Get Started"].tap()
        if app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 2) {
            acceptTerms()
        }

        button(startingWith: "No, I").tap()
        XCTAssertTrue(staticText(startingWith: "Back up your wallet").waitForExistence(timeout: 15))
        button(startingWith: "Enable").tap()
        XCTAssertTrue(app.staticTexts["Cloud Backup"].waitForExistence(timeout: 10))

        [
            "my passkey is required",
            "access to my iCloud account",
            "manually back up my 12 or 24 words",
        ].forEach { labelFragment in
            let acknowledgement = app.staticTexts.matching(
                NSPredicate(format: "label CONTAINS[c] %@", labelFragment)
            ).firstMatch
            XCTAssertTrue(acknowledgement.waitForExistence(timeout: 5))
            if !acknowledgement.isHittable {
                app.swipeUp()
            }
            acknowledgement.tap()
        }

        app.buttons["Enable Cloud Backup"].tap()
        continueNativePasskeySheetIfNeeded()

        let confirm = app.buttons["Confirm Passkey"]
        XCTAssertTrue(confirm.waitForExistence(timeout: 30))
        confirm.tap()
        continueNativePasskeySheetIfNeeded()

        XCTAssertTrue(app.staticTexts["Cloud Backup enabled"].waitForExistence(timeout: 30))
    }

    private func launchAndOpenCloudBackupDetail(scenario: String) {
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_RESET"] = "1"
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_SCENARIO"] = scenario
        app.launch()

        let restore = app.buttons["Restore with Passkey"]
        if !restore.waitForExistence(timeout: 10) {
            XCTAssertTrue(app.staticTexts["Welcome to Cove"].waitForExistence(timeout: 10))
            app.buttons["Get Started"].tap()
            acceptTerms()

            if restore.waitForExistence(timeout: 15) {
                restore.tap()
            } else {
                let manualRestore = button(startingWith: "Restore from Cove backup")
                XCTAssertTrue(manualRestore.waitForExistence(timeout: 10))
                manualRestore.tap()
                XCTAssertTrue(restore.waitForExistence(timeout: 10))
                restore.tap()
            }
        } else {
            restore.tap()
        }

        if !app.staticTexts["You’re all set"].waitForExistence(timeout: 30) {
            let manualRestore = button(startingWith: "Restore from Cove backup")
            XCTFail("restore did not complete; manual restore exists=\(manualRestore.exists)")
        }
        app.buttons["Done"].tap()

        if app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 10) {
            acceptTerms()
        }

        let openSidebar = app.buttons["app.sidebar.open"]
        XCTAssertTrue(openSidebar.waitForExistence(timeout: 30))
        openSidebar.tap()

        let settings = app.buttons["Settings"]
        XCTAssertTrue(settings.waitForExistence(timeout: 10))
        settings.tap()
        XCTAssertTrue(app.navigationBars["Settings"].waitForExistence(timeout: 10))

        let cloudBackup = app.staticTexts["Cloud Backup Enabled"]
        XCTAssertTrue(cloudBackup.waitForExistence(timeout: 15))
        cloudBackup.tap()
        XCTAssertTrue(app.navigationBars["Cloud Backup"].waitForExistence(timeout: 10))
    }

    private func acceptTerms() {
        XCTAssertTrue(app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 10))

        [
            "onboarding.terms.check.backup",
            "onboarding.terms.check.legal",
            "onboarding.terms.check.financial",
            "onboarding.terms.check.recovery",
            "onboarding.terms.check.agreement",
        ].forEach { app.buttons[$0].tap() }

        app.buttons["onboarding.terms.agree"].tap()
    }

    private var inventoryChecking: XCUIElement {
        element(identifier: "cloudBackup.inventory.checking")
    }

    private var inventoryIncomplete: XCUIElement {
        element(identifier: "cloudBackup.inventory.incomplete")
    }

    private func walletRow(recordId: String) -> XCUIElement {
        element(identifier: "cloudBackup.wallet.\(recordId)")
    }

    private func element(identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }

    private func button(startingWith labelPrefix: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }

    private func staticText(startingWith labelPrefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }

    private func continueNativePasskeySheetIfNeeded() {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let candidates = [
            app.buttons["Continue"],
            springboard.buttons["Continue"],
            app.buttons["Save a Passkey"],
            springboard.buttons["Save a Passkey"],
        ]

        for candidate in candidates where candidate.waitForExistence(timeout: 2) {
            candidate.tap()
            return
        }
    }
}
