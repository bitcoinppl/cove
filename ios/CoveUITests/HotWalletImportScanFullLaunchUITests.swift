import XCTest

/// Scans recovery words into onboarding's software wallet import, by QR and NFC, and checks every
/// scan that can't fill the words ends with the scanner closed and an alert saying why
///
/// Onboarding shows the import screen outside the app's main view, so an alert raised on the
/// app's own alert state never shows there, and the screen's dismiss does nothing at the root of
/// onboarding's embedded navigation. The QR seam (`COVE_UI_TEST_QR_SCRIPT`), the NFC seam
/// (`COVE_UI_TEST_NFC_NDEF_SCRIPT`) and the camera prompt seam (`COVE_UI_TEST_CAMERA_PROMPT`)
/// stand in for the camera, a tag and the simulator's missing camera prompt (docs/testing.md).
/// Needs no wallet and no node
final class HotWalletImportScanFullLaunchUITests: XCTestCase {
    /// A valid BIP39 phrase of 15 words, which Cove doesn't import: it takes 12 or 24
    private static let fifteenWordPhrase = (Array(repeating: "abandon", count: 14) + ["address"])
        .joined(separator: " ")

    private var app: XCUIApplication!
    private var qr: QrScript!
    private var nfc: NfcNdefScript!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        qr = QrScript()
        qr.configure(app)
        nfc = NfcNdefScript()
        nfc.configure(app)
    }

    override func tearDown() {
        qr?.remove()
        nfc?.remove()
        super.tearDown()
    }

    /// A recovery QR with 15 words, scanned from Import Wallet's toolbar, closes the scanner and
    /// says only 12 or 24 words are supported
    func testQrWithUnsupportedWordCountExplainsWhy() throws {
        openWordsImport()
        app.tapButton(named: "hotWalletImport.scanQr", until: scanner)

        try qr.show([Self.fifteenWordPhrase])

        assertExplainsUnsupportedWordCount()
    }

    /// The same QR scanned from onboarding's Scan QR code, which imports what it scans right away,
    /// explains instead of importing a wallet Cove can't take
    func testScanQrImportOfUnsupportedWordCountExplainsInsteadOfImporting() throws {
        openScanQrImport()
        XCTAssertTrue(scanner.waitForExistence(timeout: 10), "Scan QR code never opened the scanner")

        try qr.show([Self.fifteenWordPhrase])

        assertExplainsUnsupportedWordCount()
        XCTAssertFalse(
            app.staticTexts["Protect this wallet with Cloud Backup?"].exists,
            "the 15-word phrase was imported"
        )
    }

    /// An NFC tag whose text isn't recovery words says so on onboarding's import screen
    func testNfcTagWithoutRecoveryWordsExplainsWhy() throws {
        openWordsImport()
        app.tapButton(named: "hotWalletImport.scanNfc")

        try nfc.showTextRecord("not a recovery phrase")

        let explanation = app.alerts["Invalid NFC Tag"]
        XCTAssertTrue(explanation.waitForExistence(timeout: 10), "the app never said the tag holds no recovery words")
    }

    /// Don't Allow on the camera prompt that onboarding's Scan QR code raises closes the scanner and
    /// explains the camera is needed
    func testDenyingCameraAccessClosesTheScannerAndExplains() {
        app.resetAuthorizationStatus(for: .camera)
        app.launchEnvironment["COVE_UI_TEST_CAMERA_PROMPT"] = "1"

        openScanQrImport()
        denyCameraAccess()

        XCTAssertTrue(scanner.waitForNonExistence(timeout: 10), "the scanner stayed open after Don't Allow")
        let explanation = app.alerts["Camera Access is Required"]
        XCTAssertTrue(explanation.waitForExistence(timeout: 10), "the app never explained the camera is needed")
    }

    // MARK: - Steps

    /// The simulator's scanner, which shows its zoom button over the simulated camera
    private var scanner: XCUIElement {
        app.buttons["1x"]
    }

    /// Opens onboarding's 12-word import screen, the one with QR and NFC in its toolbar
    private func openWordsImport() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.startImportingExistingWallet()
        app.tapButton(startingWith: "Software wallet", until: app.button(startingWith: "Enter recovery words"))
        app.tapButton(startingWith: "Enter recovery words", until: app.button(startingWith: "12 words"))
        app.tapButton(startingWith: "12 words", until: app.navigationBars["Import Wallet"])
    }

    /// Opens onboarding's Scan QR code, which opens the scanner as it shows
    private func openScanQrImport() {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.startImportingExistingWallet()
        app.tapButton(startingWith: "Software wallet", until: app.button(startingWith: "Scan QR code"))
        app.tapButton(startingWith: "Scan QR code")
    }

    private func assertExplainsUnsupportedWordCount() {
        let explanation = app.alerts.firstMatch
        XCTAssertTrue(explanation.waitForExistence(timeout: 10), "the app never said 15 words aren't supported")

        let wordCount = explanation.staticTexts.matching(NSPredicate(format: "label CONTAINS %@", "15 words")).firstMatch
        XCTAssertTrue(wordCount.exists, "the alert doesn't name the word count: \(explanation.debugDescription)")
        XCTAssertFalse(scanner.exists, "the scanner stayed open under the alert")
    }

    /// Waits for the system camera prompt and taps Don't Allow
    private func denyCameraAccess() {
        let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        let prompt = springboard.alerts.firstMatch
        XCTAssertTrue(prompt.waitForExistence(timeout: 10), "the camera prompt never showed")

        // the system writes Don't Allow with a curly apostrophe
        prompt.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", "Don")).firstMatch.tap()
        XCTAssertTrue(prompt.waitForNonExistence(timeout: 10), "the camera prompt never closed")
    }
}
