import XCTest

/// Receives KeyTeleport transfers in the real app, with the UI test host playing the sending phone
///
/// Each test reads the receiver request off the screen as a second phone's camera would, by
/// decoding the QR code in a screenshot, and the receiver code from its text. The host encrypts the
/// response with the `keyteleport` crate, and the debug QR seam shows its frames to the app's
/// scanner (docs/testing.md). Needs the hardware signer host, which `xtask ios-ui`
/// (`just ios-ui-background`) starts for every run, and a freshly erased simulator, which xtask
/// also provides
final class KeyTeleportReceiveFullLaunchUITests: XCTestCase {
    private static let esploraUrl = "http://127.0.0.1:39001"
    private static let largestContentSize = "UICTContentSizeCategoryAccessibilityXXXL"

    /// A BIP39 test vector, a different wallet from the one onboarding imports
    private static let sentWords = "legal winner thank year wave sausage worth useful legal winner thank yellow"

    /// The master key of BIP32 test vector 1
    private static let sentXprv =
        "xprv9s21ZrQH143K3QTDL4LXw2F7HEK3wJUD2nW2nRk4stbPy6cq3jPPqjiChkVvvNKmPGJxWUtg6LnF5kejMRNNU3TGtRBeJgk33yuGBxrMPHi"

    /// A note and a password record in the shape COLDCARD's Secure Notes send them
    private static let sentNotes: [[String: String]] = [
        ["title": "Quick Note", "misc": "Attack at dawn.", "group": "Personal"],
        ["title": "Exchange", "user": "satoshi", "password": "hunter2", "site": "example.com", "misc": "Rotate yearly", "group": "Work"],
    ]

    private var app: XCUIApplication!
    private var peer: KeyTeleportPeer!
    private var signer: HardwareSignerControl!
    private var qr: QrScript!
    private var layout: ScreenReachability!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        peer = KeyTeleportPeer()
        signer = HardwareSignerControl()
        qr = QrScript()
        layout = ScreenReachability(app: app)

        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
        super.tearDown()
    }

    // MARK: - Review actions

    /// The words arrive hidden, Finish Without Importing adds no wallet, and Import opens the new one
    func testMnemonicReviewRevealsOnTapAndOnlyImportAddsTheWallet() throws {
        try launchWithWallet()

        openKeyTeleportReceive()
        try receive(.mnemonic(Self.sentWords))
        waitForReview("Recovery words received")
        assertWordsConcealed()

        layout.reachAndTap(button("Tap to reveal recovery words"), "mnemonic review: reveal")
        assertWordsShown()

        // finishing ends the transfer and goes back to Add Wallet
        layout.reachAndTap(button("Finish Without Importing"), "mnemonic review: Finish Without Importing")
        XCTAssertTrue(app.buttons["More wallet import options"].waitForExistence(timeout: 10), "Finish should return to Add Wallet")

        // had Finish imported the words, importing them now would find them already imported
        openKeyTeleportReceive()
        try receive(.mnemonic(Self.sentWords))
        waitForReview("Recovery words received")
        assertWordsConcealed()
        layout.reachAndTap(button("Import Wallet"), "mnemonic review: Import Wallet")
        try openImportedWallet()
    }

    /// The key needs Reveal, Hide conceals it again, Copy copies it, and Import opens the new wallet
    func testXprvReviewRevealHideCopyAndImport() throws {
        try launchWithWallet()

        openKeyTeleportReceive()
        try receive(.xprv(Self.sentXprv))
        waitForReview("Extended private key received")

        let key = revealedKey()
        XCTAssertFalse(key.exists, "the key must stay hidden until Reveal XPRV")

        layout.reachAndTap(button("Reveal XPRV"), "xprv review: Reveal XPRV")
        XCTAssertTrue(key.waitForExistence(timeout: 5), "Reveal XPRV should show the key")
        XCTAssertEqual(key.label, Self.sentXprv)

        layout.reachAndTap(button("Hide"), "xprv review: Hide")
        XCTAssertTrue(key.waitForNonExistence(timeout: 5), "Hide should conceal the key again")
        XCTAssertTrue(button("Reveal XPRV").firstMatch.exists, "Hide should offer Reveal XPRV again")

        layout.reachAndTap(button("Reveal XPRV"), "xprv review: Reveal XPRV again")
        XCTAssertTrue(key.waitForExistence(timeout: 5))
        layout.reachAndTap(button(startingWith: "Copy"), "xprv review: Copy")
        XCTAssertEqual(try waitForPasteboard(Self.sentXprv), Self.sentXprv, "Copy should copy the shown key")

        layout.reachAndTap(button("Import Wallet"), "xprv review: Import Wallet")
        try openImportedWallet()
    }

    // MARK: - Scanning

    /// A response too large for one QR code arrives as a multi-part BBQr, scanned out of order
    func testMultiPartBbqrResponseReachesTheReview() throws {
        try launchWithWallet()

        openKeyTeleportReceive()
        let parts = try receive(.mnemonic(Self.sentWords), minParts: 3)
        XCTAssertGreaterThanOrEqual(parts, 3, "the response must be a multi-part BBQr")

        waitForReview("Recovery words received")
        layout.reachAndTap(button("Tap to reveal recovery words"), "mnemonic review: reveal")
        assertWordsShown()
    }

    // MARK: - Largest text size

    /// The request, password, mnemonic review, imported and already imported screens keep their
    /// text and actions reachable at the largest text size in dark mode, with secrets still hidden
    func testMnemonicReceiveScreensFitAtLargestTextSizeInDarkMode() throws {
        try launchWithWalletAtLargestTextSizeInDarkMode()

        openKeyTeleportReceive()
        try receive(.mnemonic(Self.sentWords), checkingLayout: true)
        waitForReview("Recovery words received")
        assertWordsConcealed()
        layout.assertScreenFits(
            "mnemonic review",
            texts: ["Recovery words received", "Cove found a 12-word wallet. Review it below or import it directly."],
            buttons: [button("Tap to reveal recovery words"), button("Import Wallet"), button("Finish Without Importing")]
        )

        layout.reachAndTap(button("Tap to reveal recovery words"), "mnemonic review: reveal")
        assertNumberedWordsFit()
        layout.reachAndTap(button("Import Wallet"), "mnemonic review: Import Wallet")

        XCTAssertTrue(app.staticTexts["Wallet imported"].waitForExistence(timeout: 30))
        layout.assertScreenFits("imported", texts: ["Wallet imported", message(endingWith: " is ready to use in Cove.")], buttons: [button("Done")])
        layout.reachAndTap(button("Done"), "imported: Done")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))

        // the words onboarding imported are already in the app
        openKeyTeleportReceive()
        try receive(.mnemonic(XCUIApplication.testMnemonic))
        waitForReview("Recovery words received")
        layout.reachAndTap(button("Import Wallet"), "mnemonic review: Import Wallet")

        XCTAssertTrue(app.staticTexts["Wallet already imported"].waitForExistence(timeout: 30))
        layout.assertScreenFits(
            "already imported",
            texts: ["Wallet already imported", message(endingWith: " is already available in Cove.")],
            buttons: [button("Open Wallet")]
        )
        layout.reachAndTap(button("Open Wallet"), "already imported: Open Wallet")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    /// The private key and message reviews keep their text and actions reachable at the largest text
    /// size in dark mode, with the key hidden until revealed
    func testXprvAndMessageReviewsFitAtLargestTextSizeInDarkMode() throws {
        try launchWithWalletAtLargestTextSizeInDarkMode()

        openKeyTeleportReceive()
        try receive(.xprv(Self.sentXprv))
        waitForReview("Extended private key received")
        XCTAssertFalse(revealedKey().exists, "the key must stay hidden until Reveal XPRV")
        layout.assertScreenFits(
            "xprv review",
            texts: [
                "Extended private key received",
                "Import this key as a hot wallet, or reveal it only when you are ready to handle the private key.",
                "Reveal only if you are ready to handle this private key.",
            ],
            buttons: [button("Reveal XPRV"), button("Import Wallet"), button("Finish Without Importing")]
        )

        layout.reachAndTap(button("Reveal XPRV"), "xprv review: Reveal XPRV")
        layout.assertScreenFits("revealed xprv", texts: [Self.sentXprv], buttons: [button(startingWith: "Copy"), button("Hide")])
        layout.reachAndTap(button("Finish Without Importing"), "xprv review: Finish Without Importing")

        openKeyTeleportReceive()
        try receive(.notes(Self.sentNotes))
        waitForReview("Messages received")
        layout.assertScreenFits(
            "message review",
            texts: [
                "Messages received",
                "This transfer contains text, not a wallet. Cove has displayed it exactly as received.",
                "Quick Note", "Personal", "MESSAGE", "Attack at dawn.",
                "Exchange", "Work", "USERNAME", "satoshi", "PASSWORD", "hunter2", "WEBSITE", "example.com", "NOTES", "Rotate yearly",
            ],
            buttons: [button("Done")]
        )
        layout.reachAndTap(button("Done"), "message review: Done")
        XCTAssertTrue(app.buttons["More wallet import options"].waitForExistence(timeout: 10), "Done should return to Add Wallet")
    }

    // MARK: - Launching

    private func launchWithWallet() throws {
        try signer.requireHealthy()
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
    }

    /// Finishes onboarding at the default size, since only the KeyTeleport screens are under test,
    /// then relaunches at the largest size in dark mode
    private func launchWithWalletAtLargestTextSizeInDarkMode() throws {
        try launchWithWallet()
        app.terminate()

        try signer.setSimulatorAppearance(dark: true)
        app.launchArguments += ["-UIPreferredContentSizeCategoryName", Self.largestContentSize]
        app.launch(expecting: app.element("selectedWallet.balance"))
    }

    // MARK: - Receiving

    /// Opens Receive by KeyTeleport from Add Wallet, opening Add Wallet from the sidebar first unless
    /// it is already showing
    private func openKeyTeleportReceive() {
        let moreOptions = app.buttons["More wallet import options"]
        if !moreOptions.waitForExistence(timeout: 2) {
            app.tapButton(named: "app.sidebar.open")
            app.tapButton(named: "Add Wallet")
        }

        XCTAssertTrue(moreOptions.waitForExistence(timeout: 10))
        moreOptions.tap()
        app.tapButton(named: "Key Teleport")
        XCTAssertTrue(button("Tap to show QR code").firstMatch.waitForExistence(timeout: 15), "the receive request never showed")
    }

    /// Reads the request off the screen, has the host answer it, scans the answer and unlocks it
    /// with the password; returns how many QR frames the answer took
    @discardableResult
    private func receive(_ payload: KeyTeleportPeer.Payload, minParts: Int = 1, checkingLayout: Bool = false) throws -> Int {
        let (request, code) = readReceiverRequest(checkingLayout: checkingLayout)
        let response = try peer.send(payload, to: request, code: code, minParts: minParts)

        layout.reachAndTap(button("Scan Sender Response"), "receive request: Scan Sender Response")

        // the camera can catch an animated QR at any frame
        try qr.show(Array(response.parts.reversed()))

        let field = app.secureTextFields["Teleport Password"]
        XCTAssertTrue(field.waitForExistence(timeout: 20), "the scanned response never asked for the password")
        if checkingLayout {
            layout.assertScreenFits(
                "password",
                texts: [],
                buttons: [button("Unlock")],
                controls: [app.secureTextFields.matching(identifier: "Teleport Password"), button("Show password")]
            )
            layout.assertNoTruncatedText("password")
        }

        layout.reachAndTap(app.secureTextFields.matching(identifier: "Teleport Password"), "password: field").typeText(response.password)
        layout.reachAndTap(button("Unlock"), "password: Unlock")

        return response.parts.count
    }

    /// Reads the request QR code and the receiver code off the screen, checking they show only one
    /// at a time
    private func readReceiverRequest(checkingLayout: Bool) -> (request: String, code: String) {
        let code = receiverCode()
        if checkingLayout {
            layout.assertScreenFits(
                "receive request",
                texts: ["Receive by KeyTeleport"],
                buttons: [button("Tap to show QR code"), button("Tap to show receiver code"), button("Scan Sender Response")]
            )
        }

        guard let pair = layout.readRevealPair(
            qrHint: "Tap to show QR code",
            codeHint: "Tap to show receiver code",
            code: code,
            "receive request"
        ) else { return ("", "") }

        XCTAssertTrue(pair.qr.hasPrefix("B$2R"), "the QR code should hold a KeyTeleport receiver request, got \(pair.qr.prefix(8))")
        return (pair.qr, pair.code)
    }

    private func waitForReview(_ title: String) {
        XCTAssertTrue(app.staticTexts[title].waitForExistence(timeout: 30), "the review '\(title)' never showed")
    }

    /// Opens the wallet the review imported from the Wallet imported screen
    private func openImportedWallet() throws {
        let imported = app.staticTexts["Wallet imported"]
        XCTAssertTrue(imported.waitForExistence(timeout: 30), "Import Wallet should import a new wallet")

        let message = app.staticTexts.matching(NSPredicate(format: "label ENDSWITH %@", " is ready to use in Cove.")).firstMatch
        XCTAssertTrue(message.exists)
        let name = String(message.label.dropLast(" is ready to use in Cove.".count))

        layout.reachAndTap(button("Done"), "imported: Done")
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30), "Done should open the imported wallet")
        XCTAssertTrue(app.staticText(containing: name).waitForExistence(timeout: 10), "Done should open '\(name)', the imported wallet")
    }

    // MARK: - Secrets on screen

    private func assertWordsConcealed() {
        for word in Set(Self.sentWords.split(separator: " ")) {
            XCTAssertFalse(app.staticTexts[String(word)].exists, "the words must stay hidden until the user reveals them")
        }

        let hint = "Tap to reveal recovery words"
        layout.assertReachable(button(hint), "review: '\(hint)'", needsHit: true).assertConceals(hint: hint, "the recovery words")
    }

    /// Each word fits on screen beside its number, found by position since the test words repeat,
    /// so a clipped second 'legal' can't pass on the first
    private func assertNumberedWordsFit() {
        let window = app.windows.firstMatch.frame
        for (index, word) in Self.sentWords.split(separator: " ").map(String.init).enumerated() {
            let name = "revealed words: word \(index + 1)"
            let numbers = app.staticTexts.matching(NSPredicate(format: "label == %@", "\(index + 1)"))
            layout.scrollDown(untilExists: numbers)
            layout.assertReachable(numbers, name, needsHit: false)
            let shown = app.staticTexts.matching(NSPredicate(format: "label == %@", word)).allElementsBoundByIndex.first { candidate in
                numbers.allElementsBoundByIndex.contains { isBeside(candidate.frame, $0.frame) }
            }

            guard let shown else {
                XCTFail("\(name) should be '\(word)'")
                continue
            }

            XCTAssertTrue(window.contains(shown.frame), "\(name), '\(word)', runs off screen at \(shown.frame)")
            layout.assertDrawn(word, in: shown, name)
            layout.assertNoTruncatedText(name)
        }
    }

    /// Each word must show beside its number, so a review that swaps two words, or shows one
    /// repeated word in place of another, fails
    private func assertWordsShown() {
        for (index, word) in Self.sentWords.split(separator: " ").map(String.init).enumerated() {
            let number = app.staticTexts.matching(NSPredicate(format: "label == %@", "\(index + 1)"))
            let words = app.staticTexts.matching(NSPredicate(format: "label == %@", word))
            XCTAssertTrue(words.firstMatch.waitForExistence(timeout: 5), "the revealed words should include '\(word)'")

            let numbered = number.allElementsBoundByIndex.contains { number in
                words.allElementsBoundByIndex.contains { isBeside($0.frame, number.frame) }
            }

            XCTAssertTrue(numbered, "word \(index + 1) should be '\(word)'")
        }
    }

    /// Whether the word sits in the same cell as the number, just to its right
    private func isBeside(_ word: CGRect, _ number: CGRect) -> Bool {
        abs(word.midY - number.midY) < 4 && word.minX > number.maxX && word.minX - number.maxX < 40
    }

    private func revealedKey() -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label BEGINSWITH 'xprv' OR label BEGINSWITH 'tprv'")).firstMatch
    }

    private func receiverCode() -> XCUIElement {
        app.staticTexts.matching(NSPredicate(format: "label MATCHES %@", "^[0-9]{2}( [0-9]{2}){3}$")).firstMatch
    }

    /// The app sets the pasteboard asynchronously from the tap, so read until it holds the key
    private func waitForPasteboard(_ expected: String) throws -> String {
        let deadline = Date().addingTimeInterval(5)
        var text = ""
        while Date() < deadline {
            text = try signer.simulatorPasteboard().trimmingCharacters(in: .whitespacesAndNewlines)
            if text == expected { return text }

            Thread.sleep(forTimeInterval: 0.3)
        }

        return text
    }

    // MARK: - Queries

    private func button(_ label: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label == %@", label))
    }

    /// A result screen's message, which starts with the wallet's name
    private func message(endingWith suffix: String) -> String {
        let message = app.staticTexts.matching(NSPredicate(format: "label ENDSWITH %@", suffix)).firstMatch
        XCTAssertTrue(message.waitForExistence(timeout: 5), "the message ending '\(suffix)' is missing")
        return message.label
    }

    private func button(startingWith prefix: String) -> XCUIElementQuery {
        app.buttons.matching(NSPredicate(format: "label BEGINSWITH %@", prefix))
    }
}
