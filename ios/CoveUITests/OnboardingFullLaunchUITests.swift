import UIKit
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
        launchApp()

        XCTAssertTrue(app.staticTexts["Welcome to Cove"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["Get Started"].exists)
    }

    func testSoftwareWalletImportShowsTermsBeforeWalletSelection() {
        launchApp()

        reachStorageChoices()
        openSoftwareWalletImport()

        importKnownEmptyMainnetWalletWords()
        tap(app.buttons["Not Now"], until: app.staticTexts["Terms & Conditions"])
        acceptTermsAndContinue()

        XCTAssertTrue(app.staticTexts["Multiple wallets found, please choose one"].waitForExistence(timeout: 30))
    }

    func testCanReachExistingWalletStorageChoices() {
        launchApp()

        reachStorageChoices()

        XCTAssertTrue(app.staticTexts["How do you store your Bitcoin?"].exists)
        XCTAssertTrue(button(startingWith: "On an exchange").exists)
        XCTAssertTrue(button(startingWith: "Hardware wallet").exists)
        XCTAssertTrue(button(startingWith: "Software wallet").exists)
    }

    func testExistingUserCanGoBackToBitcoinChoice() {
        launchApp()

        getStarted()
        tap(button(startingWith: "Yes, I have Bitcoin"), until: app.staticTexts["How do you store your Bitcoin?"])
        tap(app.buttons["Back"], until: app.staticTexts["Do you already have Bitcoin?"])

        XCTAssertTrue(button(startingWith: "No, I").exists)
        XCTAssertTrue(button(startingWith: "Yes, I have Bitcoin").exists)
    }

    func testExchangeUserCanReachFundingAfterBackup() {
        launchApp()

        reachStorageChoices()
        tap(button(startingWith: "On an exchange"), until: backupWalletTitle(), timeout: 15, attempts: 1)
        assertBackupWallet(titlePrefix: "Back up your wallet before funding it")

        tap(button(startingWith: "Show Words"), until: app.staticTexts["Your Recovery Words"])

        tap(app.buttons["I Saved These Words"], until: button(startingWith: "Saved"))
        assertBackupWallet(titlePrefix: "Back up your wallet before funding it", recoveryWordsSaved: true)

        tap(app.buttons["Continue"], until: app.staticTexts["Your wallet is ready to fund"], timeout: 15, attempts: 1)
        XCTAssertTrue(app.staticTexts["Deposit address"].waitForExistence(timeout: 30))

        // the "Copied" label reverts after 2 seconds, which a slow accessibility snapshot can miss,
        // so check the pasteboard write itself; reading changeCount does not trigger the paste prompt
        let pasteboardChangeCount = UIPasteboard.general.changeCount
        app.buttons["Copy Address"].tap()

        let copied = expectation(for: NSPredicate { _, _ in
            UIPasteboard.general.changeCount != pasteboardChangeCount
        }, evaluatedWith: nil)
        wait(for: [copied], timeout: 10)
    }

    func testHardwareWalletUserCanReachImportChoices() {
        launchApp()

        reachStorageChoices()
        openHardwareWalletImport()

        XCTAssertTrue(button(startingWith: "Scan export QR").exists)
        XCTAssertTrue(button(startingWith: "Import export file").exists)
        XCTAssertTrue(button(startingWith: "Scan with NFC").exists)
    }

    func testHardwareWalletUserCanOpenQrScanner() {
        launchApp()

        reachStorageChoices()
        openHardwareWalletImport()
        tap(button(startingWith: "Scan export QR"), until: app.navigationBars["Scan Hardware QR"])

        XCTAssertTrue(app.staticTexts["Scan Wallet Export QR Code"].waitForExistence(timeout: 10))
    }

    func testHardwareWalletUserCanOpenNfcScanner() {
        launchApp()

        reachStorageChoices()
        openHardwareWalletImport()
        tap(button(startingWith: "Scan with NFC"), until: app.staticTexts["Scan your hardware wallet with NFC"])

        XCTAssertTrue(app.buttons["Start NFC Scan"].exists)
    }

    func testSoftwareWalletUserCanCreateNewWallet() {
        launchApp()

        reachStorageChoices()
        openSoftwareWalletImport()
        tap(app.buttons["Create a new wallet instead"], until: backupWalletTitle(), timeout: 15, attempts: 1)

        assertBackupWallet(titlePrefix: "Back up your wallet")
        viewRecoveryWords()
    }

    func testSoftwareWalletUserCanReachImportChoices() {
        launchApp()

        reachStorageChoices()
        openSoftwareWalletImport()
        XCTAssertTrue(button(startingWith: "Enter recovery words").exists)
        XCTAssertTrue(button(startingWith: "Scan QR code").exists)
    }

    func testSoftwareWalletImportSuggestionsAreAnchoredToKeyboard() {
        launchApp()

        reachStorageChoices()
        openSoftwareWalletImport()
        tap(button(startingWith: "Enter recovery words"), until: app.staticTexts["How many words do you have?"])
        tap(button(startingWith: "12 words"), until: app.navigationBars["Import Wallet"])

        let firstField = app.textFields["hotWalletImport.word.1"]
        XCTAssertTrue(firstField.waitForExistence(timeout: 10))
        firstField.tap()

        let keyboardTutorialContinue = app.buttons["Continue"]
        if keyboardTutorialContinue.waitForExistence(timeout: 1) {
            keyboardTutorialContinue.tap()
        }

        let keyboard = app.keyboards.firstMatch
        XCTAssertTrue(keyboard.waitForExistence(timeout: 10))
        firstField.typeText("a")

        let firstSuggestion = app.buttons["abandon"]
        XCTAssertTrue(firstSuggestion.waitForExistence(timeout: 5))

        let suggestions = app.descendants(matching: .any)["hotWalletImport.suggestions"]
        XCTAssertTrue(suggestions.waitForExistence(timeout: 5))

        let screenshot = XCTAttachment(screenshot: app.screenshot())
        screenshot.name = "Import wallet word suggestions"
        screenshot.lifetime = .keepAlways
        add(screenshot)

        let accessoryGap = keyboard.frame.minY - suggestions.frame.maxY
        XCTAssertLessThanOrEqual(
            accessoryGap,
            20,
            "expected word suggestions to be attached to the keyboard, got a \(accessoryGap)-point gap"
        )
    }

    func testSoftwareWalletUserCanOpenQrScanner() {
        launchApp()

        reachStorageChoices()
        openSoftwareWalletImport()
        tap(button(startingWith: "Scan QR code"), until: app.buttons["1x"], app.alerts["Camera Access Required"])
    }

    func testSoftwareWalletUserCanImportKnownWords() {
        launchApp()

        reachStorageChoices()
        openSoftwareWalletImport()

        importKnownEmptyMainnetWalletWords()
        tap(app.buttons["Not Now"], until: app.staticTexts["Terms & Conditions"])
        acceptTermsAndContinue()
        chooseNativeImportedWalletFromSelectionSheet()

        XCTAssertTrue(app.staticTexts["Transactions"].waitForExistence(timeout: 30))
        XCTAssertTrue(staticText(containing: "0 BTC").waitForExistence(timeout: 30))
        XCTAssertTrue(app.staticTexts["Checking wallet history"].waitForExistence(timeout: 5))
        XCTAssertFalse(app.staticTexts["No transactions"].waitForExistence(timeout: 1))
    }

    func testSoftwareWalletUserCanImportKnownLegacyWalletWithHistory() {
        launchApp()

        reachStorageChoices()
        openSoftwareWalletImport()

        importKnownEmptyMainnetWalletWords()
        tap(app.buttons["Not Now"], until: app.staticTexts["Terms & Conditions"])
        acceptTermsAndContinue()
        chooseLegacyImportedWalletFromSelectionSheet()

        XCTAssertTrue(app.staticTexts["Transactions"].waitForExistence(timeout: 30))
        XCTAssertTrue(staticText(containing: "0 BTC").waitForExistence(timeout: 30))
        XCTAssertTrue(app.staticTexts["Checking wallet history"].waitForNonExistence(timeout: 45))
        XCTAssertFalse(app.staticTexts["No transactions"].exists)
    }

    func testNewUserCanReachBackupWallet() {
        launchApp()

        reachBackupWallet()

        assertBackupWallet(titlePrefix: "Back up your wallet")
    }

    func testNewUserCanViewRecoveryWordsAndReturn() {
        launchApp()

        reachBackupWallet()
        viewRecoveryWords()

        tap(app.buttons["Back"], until: backupWalletTitle())
        XCTAssertTrue(button(startingWith: "Show Words").exists)
    }

    func testNewUserCloudBackupDetailsCanCancel() {
        launchApp()

        reachBackupWallet()
        tap(button(startingWith: "Enable"), until: app.staticTexts["Cloud Backup"])
        XCTAssertTrue(app.buttons["Cancel"].exists)

        tap(app.buttons["Cancel"], until: backupWalletTitle())
        XCTAssertTrue(button(startingWith: "Enable").exists)
    }

    /// Launches the app and waits for onboarding, relaunching once if startup timed out
    ///
    /// A freshly erased CI simulator can take over a minute to launch the app, which trips the
    /// 10 second bootstrap watchdog; the timeout screen asks the user to force-quit and retry,
    /// so do that once and still fail when a warm launch also times out
    private func launchApp() {
        // the Cloud Backup target builds this class too, and runs each class's tests in one
        // simulator session, so its host app must wipe the last test's wallets; Cove ignores it
        app.launchEnvironment["COVE_CLOUD_BACKUP_UI_RESET"] = "1"
        app.launch()

        guard waitForOnboardingOrStartupTimeout() == .startupTimedOut else { return }

        app.terminate()
        app.launch()

        if waitForOnboardingOrStartupTimeout() == .startupTimedOut {
            XCTFail("App startup timed out on relaunch")
        }
    }

    private enum LaunchOutcome {
        case onboarding
        case startupTimedOut
        case unknown
    }

    private func waitForOnboardingOrStartupTimeout() -> LaunchOutcome {
        let welcome = app.staticTexts["Welcome to Cove"]
        let startupTimedOut = staticText(containing: "App startup timed out")

        // the watchdog allows up to 20 seconds while a migration runs
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            if welcome.exists { return .onboarding }
            if startupTimedOut.exists { return .startupTimedOut }

            Thread.sleep(forTimeInterval: 0.5)
        }

        return .unknown
    }

    private func reachBackupWallet() {
        getStarted()
        tap(button(startingWith: "No, I"), until: backupWalletTitle(), timeout: 15, attempts: 1)

        assertBackupWallet(titlePrefix: "Back up your wallet")
    }

    private func reachStorageChoices() {
        getStarted()
        tap(button(startingWith: "Yes, I have Bitcoin"), until: app.staticTexts["How do you store your Bitcoin?"])
    }

    private func openSoftwareWalletImport() {
        tap(button(startingWith: "Software wallet"), until: app.staticTexts["Import your software wallet"])
    }

    private func openHardwareWalletImport() {
        tap(button(startingWith: "Hardware wallet"), until: app.staticTexts["Import your hardware wallet"])
    }

    private func backupWalletTitle() -> XCUIElement {
        staticText(startingWith: "Back up your wallet")
    }

    private func assertBackupWallet(titlePrefix: String, recoveryWordsSaved: Bool = false) {
        XCTAssertTrue(staticText(startingWith: titlePrefix).waitForExistence(timeout: 15))
        XCTAssertTrue(button(startingWith: recoveryWordsSaved ? "Saved" : "Show Words").exists)
        XCTAssertTrue(button(startingWith: "Enable").exists)
    }

    private func viewRecoveryWords() {
        tap(button(startingWith: "Show Words"), until: app.staticTexts["Your Recovery Words"])
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

        tap(app.buttons["onboarding.terms.agree"], until: app.staticTexts["Multiple wallets found, please choose one"], timeout: 30, attempts: 1)
    }

    /// Leaves the welcome screen, accepting the terms when they show
    private func getStarted() {
        tap(app.buttons["Get Started"], until: app.staticTexts["Terms & Conditions"], app.staticTexts["Do you already have Bitcoin?"])
        acceptTermsIfNeeded()
    }

    /// Taps a button until one of the screens it leads to shows
    ///
    /// On a slow CI simulator a tap can be sent before the app handles input and do nothing, so
    /// the tap repeats while the button is still hittable and no destination has shown; a
    /// navigation that is only slow leaves the button unhittable, so it isn't tapped twice.
    /// Buttons that create a wallet, switch wallets or finish onboarding pass `attempts: 1`:
    /// they stay hittable while their request runs, so a repeat would send a second request
    private func tap(
        _ button: XCUIElement,
        until destinations: XCUIElement...,
        timeout: TimeInterval = 10,
        attempts: Int = 3,
        file: StaticString = #filePath,
        line: UInt = #line
    ) {
        guard button.waitForExistence(timeout: 10) else {
            XCTFail("Timed out waiting for \(button)", file: file, line: line)
            return
        }

        let inlineError = app.staticTexts["onboarding.inlineMessage"]
        for _ in 0 ..< attempts {
            button.tap()
            if waitForAny(of: destinations, timeout: timeout) { return }

            // a failed request shows its error, which another tap would clear and hide
            if inlineError.exists {
                XCTFail("Tapping \(button) showed \"\(inlineError.label)\"", file: file, line: line)
                return
            }

            // a button that stopped being hittable is mid-navigation, so give it one more wait
            // instead of another tap; a still-hittable button after the last attempt fails here
            guard button.exists, button.isHittable else {
                if waitForAny(of: destinations, timeout: timeout) { return }
                break
            }
        }

        XCTFail("Tapping \(button) never showed \(destinations)", file: file, line: line)
    }

    private func waitForAny(of elements: [XCUIElement], timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            for element in elements where element.exists {
                return true
            }

            Thread.sleep(forTimeInterval: 0.5)
        }

        return false
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

        tap(app.buttons["onboarding.terms.agree"], until: app.staticTexts["Do you already have Bitcoin?"], attempts: 1)
    }

    private func chooseNativeImportedWalletFromSelectionSheet() {
        XCTAssertTrue(app.staticTexts["Multiple wallets found, please choose one"].waitForExistence(timeout: 30))
        XCTAssertTrue(button(startingWith: "Keep Current").exists)
        XCTAssertTrue(app.staticTexts["Wrapped Segwit"].exists || app.buttons["Wrapped Segwit"].exists)
        XCTAssertTrue(app.staticTexts["Legacy"].exists || app.buttons["Legacy"].exists)
        tap(button(startingWith: "Keep Current"), until: app.staticTexts["Transactions"], timeout: 30, attempts: 1)
    }

    private func chooseLegacyImportedWalletFromSelectionSheet() {
        XCTAssertTrue(app.staticTexts["Multiple wallets found, please choose one"].waitForExistence(timeout: 30))
        tap(button(startingWith: "Legacy"), until: app.staticTexts["Transactions"], timeout: 30, attempts: 1)
    }

    private func importKnownEmptyMainnetWalletWords() {
        tap(button(startingWith: "Enter recovery words"), until: app.staticTexts["How many words do you have?"])
        tap(button(startingWith: "12 words"), until: app.navigationBars["Import Wallet"])

        for (index, word) in knownEmptyMainnetMnemonic.enumerated() {
            let field = app.textFields["hotWalletImport.word.\(index + 1)"]
            XCTAssertTrue(field.waitForExistence(timeout: 10))
            field.tap()
            field.typeText(word == "about" ? "abou" : "aba")
        }

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

    private func staticText(startingWith labelPrefix: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH %@", labelPrefix)).firstMatch
    }

    private func staticText(containing text: String) -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    private func element(identifier: String) -> XCUIElement {
        app.buttons[identifier]
    }
}
