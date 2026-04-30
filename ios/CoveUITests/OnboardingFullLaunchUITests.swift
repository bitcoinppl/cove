import XCTest

final class OnboardingFullLaunchUITests: XCTestCase {
    private var app: XCUIApplication!
    private let knownEmptyMainnetMnemonic = Array(repeating: "abandon", count: 11) + ["about"]

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

    func testSoftwareWalletImportShowsTermsBeforeWalletSelection() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "Software wallet").tap()

        XCTAssertTrue(app.staticTexts["What would you like to do?"].waitForExistence(timeout: 10))
        button(startingWith: "Import existing wallet").tap()

        importKnownEmptyMainnetWalletWords()
        app.buttons["Not Now"].tap()
        acceptTermsAndContinue()

        XCTAssertTrue(app.staticTexts["Multiple wallets found, please choose one"].waitForExistence(timeout: 30))
    }

    func testCanReachExistingWalletStorageChoices() {
        app.launch()

        reachStorageChoices()

        XCTAssertTrue(app.staticTexts["How do you store your Bitcoin?"].exists)
        XCTAssertTrue(button(startingWith: "On an exchange").exists)
        XCTAssertTrue(button(startingWith: "Hardware wallet").exists)
        XCTAssertTrue(button(startingWith: "Software wallet").exists)
    }

    func testExistingUserCanGoBackToBitcoinChoice() {
        app.launch()

        app.buttons["Get Started"].tap()
        acceptTermsIfNeeded()
        button(startingWith: "Yes, I have Bitcoin").tap()
        app.buttons["Back"].tap()

        XCTAssertTrue(app.staticTexts["Do you already have Bitcoin?"].waitForExistence(timeout: 10))
        XCTAssertTrue(button(startingWith: "No, I").exists)
        XCTAssertTrue(button(startingWith: "Yes, I have Bitcoin").exists)
    }

    func testExchangeUserCanReachFundingAfterBackup() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "On an exchange").tap()
        assertBackupWallet(titlePrefix: "Back up your wallet before funding it")

        button(startingWith: "Show Words").tap()
        XCTAssertTrue(app.staticTexts["Your Recovery Words"].waitForExistence(timeout: 10))

        app.buttons["I Saved These Words"].tap()
        assertBackupWallet(titlePrefix: "Back up your wallet before funding it", recoveryWordsSaved: true)

        app.buttons["Continue"].tap()

        XCTAssertTrue(app.staticTexts["Your wallet is ready to fund"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["Loading deposit address"].waitForExistence(timeout: 10) || app.staticTexts["Deposit address"].exists)
    }

    func testHardwareWalletUserCanReachImportChoices() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "Hardware wallet").tap()

        XCTAssertTrue(app.staticTexts["Import your hardware wallet"].waitForExistence(timeout: 10))
        XCTAssertTrue(button(startingWith: "Scan export QR").exists)
        XCTAssertTrue(button(startingWith: "Import export file").exists)
        XCTAssertTrue(button(startingWith: "Scan with NFC").exists)
    }

    func testHardwareWalletUserCanOpenQrScanner() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "Hardware wallet").tap()
        button(startingWith: "Scan export QR").tap()

        XCTAssertTrue(app.navigationBars["Scan Hardware QR"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["Scan Wallet Export QR Code"].waitForExistence(timeout: 10))
    }

    func testHardwareWalletUserCanOpenNfcScanner() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "Hardware wallet").tap()
        button(startingWith: "Scan with NFC").tap()

        XCTAssertTrue(app.staticTexts["Scan your hardware wallet with NFC"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Start NFC Scan"].exists)
    }

    func testSoftwareWalletUserCanCreateNewWallet() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "Software wallet").tap()

        XCTAssertTrue(app.staticTexts["What would you like to do?"].waitForExistence(timeout: 10))
        button(startingWith: "Create a new wallet").tap()

        assertBackupWallet(titlePrefix: "Back up your wallet")
        viewRecoveryWords()
    }

    func testSoftwareWalletUserCanReachImportChoices() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "Software wallet").tap()

        XCTAssertTrue(app.staticTexts["What would you like to do?"].waitForExistence(timeout: 10))
        button(startingWith: "Import existing wallet").tap()

        XCTAssertTrue(app.staticTexts["Import your software wallet"].waitForExistence(timeout: 10))
        XCTAssertTrue(button(startingWith: "Enter recovery words").exists)
        XCTAssertTrue(button(startingWith: "Scan QR code").exists)
    }

    func testSoftwareWalletUserCanOpenQrScanner() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "Software wallet").tap()

        XCTAssertTrue(app.staticTexts["What would you like to do?"].waitForExistence(timeout: 10))
        button(startingWith: "Import existing wallet").tap()
        button(startingWith: "Scan QR code").tap()

        assertQrScannerVisible()
    }

    func testSoftwareWalletUserCanImportKnownWords() {
        app.launch()

        reachStorageChoices()
        button(startingWith: "Software wallet").tap()

        XCTAssertTrue(app.staticTexts["What would you like to do?"].waitForExistence(timeout: 10))
        button(startingWith: "Import existing wallet").tap()

        importKnownEmptyMainnetWalletWords()
        app.buttons["Not Now"].tap()
        acceptTermsAndContinue()
        chooseNativeImportedWalletFromSelectionSheet()

        XCTAssertTrue(app.staticTexts["Transactions"].waitForExistence(timeout: 30))
        XCTAssertTrue(staticText(containing: "0 sats").waitForExistence(timeout: 30))
    }

    func testNewUserCanReachBackupWallet() {
        app.launch()

        reachBackupWallet()

        assertBackupWallet(titlePrefix: "Back up your wallet")
    }

    func testNewUserCanViewRecoveryWordsAndReturn() {
        app.launch()

        reachBackupWallet()
        button(startingWith: "Show Words").tap()

        XCTAssertTrue(app.staticTexts["Your Recovery Words"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["I Saved These Words"].exists)

        app.buttons["Back"].tap()

        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Back up your wallet")).firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(button(startingWith: "Show Words").exists)
    }

    func testNewUserCloudBackupDetailsCanCancel() {
        app.launch()

        reachBackupWallet()
        button(startingWith: "Enable").tap()

        XCTAssertTrue(app.staticTexts["Cloud Backup"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Cancel"].exists)

        app.buttons["Cancel"].tap()

        XCTAssertTrue(app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", "Back up your wallet")).firstMatch.waitForExistence(timeout: 10))
        XCTAssertTrue(button(startingWith: "Enable").exists)
    }

    private func reachBackupWallet() {
        tapButton(named: "Get Started")
        acceptTermsIfNeeded()
        tapButton(startingWith: "No, I")

        assertBackupWallet(titlePrefix: "Back up your wallet")
    }

    private func reachStorageChoices() {
        tapButton(named: "Get Started")
        acceptTermsIfNeeded()
        tapButton(startingWith: "Yes, I have Bitcoin")
        tapButton(startingWith: "Use another wallet")

        XCTAssertTrue(app.staticTexts["How do you store your Bitcoin?"].waitForExistence(timeout: 10))
    }

    private func assertBackupWallet(titlePrefix: String, recoveryWordsSaved: Bool = false) {
        XCTAssertTrue(staticText(startingWith: titlePrefix).waitForExistence(timeout: 15))
        XCTAssertTrue(button(startingWith: recoveryWordsSaved ? "Saved" : "Show Words").exists)
        XCTAssertTrue(button(startingWith: "Enable").exists)
    }

    private func viewRecoveryWords() {
        button(startingWith: "Show Words").tap()
        XCTAssertTrue(app.staticTexts["Your Recovery Words"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["I Saved These Words"].exists)
    }

    private func acceptTermsAndContinue() {
        XCTAssertTrue(app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 10))

        [
            "onboarding.terms.check.backup",
            "onboarding.terms.check.legal",
            "onboarding.terms.check.financial",
            "onboarding.terms.check.recovery",
            "onboarding.terms.check.agreement",
        ].forEach { identifier in
            element(identifier: identifier).tap()
        }

        app.buttons["onboarding.terms.agree"].tap()
    }

    private func acceptTermsIfNeeded() {
        guard app.staticTexts["Terms & Conditions"].waitForExistence(timeout: 2) else { return }

        [
            "onboarding.terms.check.backup",
            "onboarding.terms.check.legal",
            "onboarding.terms.check.financial",
            "onboarding.terms.check.recovery",
            "onboarding.terms.check.agreement",
        ].forEach { identifier in
            element(identifier: identifier).tap()
        }

        app.buttons["onboarding.terms.agree"].tap()
    }

    private func chooseNativeImportedWalletFromSelectionSheet() {
        XCTAssertTrue(app.staticTexts["Multiple wallets found, please choose one"].waitForExistence(timeout: 30))
        XCTAssertTrue(app.buttons["Keep Current"].exists)
        XCTAssertTrue(app.staticTexts["Wrapped Segwit"].exists || app.buttons["Wrapped Segwit"].exists)
        XCTAssertTrue(app.staticTexts["Legacy"].exists || app.buttons["Legacy"].exists)
        app.buttons["Keep Current"].tap()
        XCTAssertTrue(app.staticTexts["Transactions"].waitForExistence(timeout: 30))
    }

    private func assertQrScannerVisible() {
        XCTAssertTrue(app.buttons["1x"].waitForExistence(timeout: 10) || app.alerts["Camera Access Required"].waitForExistence(timeout: 1))
    }

    private func importKnownEmptyMainnetWalletWords() {
        button(startingWith: "Enter recovery words").tap()
        XCTAssertTrue(app.staticTexts["How many words do you have?"].waitForExistence(timeout: 10))
        button(startingWith: "12 words").tap()
        XCTAssertTrue(app.navigationBars["Import Wallet"].waitForExistence(timeout: 10))

        for (index, word) in knownEmptyMainnetMnemonic.enumerated() {
            let field = app.textFields["hotWalletImport.word.\(index + 1)"]
            XCTAssertTrue(field.waitForExistence(timeout: 10))
            field.tap()
            field.typeText(word)
        }

        dismissKeyboardIfVisible()
        app.buttons["hotWalletImport.import"].tap()

        XCTAssertTrue(app.staticTexts["Protect this wallet with Cloud Backup?"].waitForExistence(timeout: 20))
    }

    private func dismissKeyboardIfVisible() {
        if app.keyboards.buttons["return"].exists {
            app.keyboards.buttons["return"].tap()
            return
        }

        if app.keyboards.buttons["Done"].exists {
            app.keyboards.buttons["Done"].tap()
        }
    }

    private func button(startingWith labelPrefix: String) -> XCUIElement {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }

    private func tapButton(named name: String) {
        let button = app.buttons[name]
        guard button.waitForExistence(timeout: 10) else {
            XCTFail("Timed out waiting for button '\(name)'")
            return
        }

        button.tap()
    }

    private func tapButton(startingWith labelPrefix: String) {
        let button = button(startingWith: labelPrefix)
        guard button.waitForExistence(timeout: 10) else {
            XCTFail("Timed out waiting for button starting with '\(labelPrefix)'")
            return
        }

        button.tap()
    }

    private func staticText(startingWith labelPrefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }

    private func staticText(containing text: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    private func element(identifier: String) -> XCUIElement {
        app.descendants(matching: .any)[identifier]
    }
}
