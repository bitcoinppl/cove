import XCTest

/// Steps through the hardware wallet import and TAPSIGNER screens that several full-launch suites
/// share
extension XCUIApplication {
    /// Opens the hardware wallet export scanner from the welcome screen
    func openHardwareExportScanner() {
        startImportingExistingWallet()
        tapButton(startingWith: "Hardware wallet", until: button(startingWith: "Scan export QR"))
        tapButton(startingWith: "Scan export QR")
    }

    /// Opens onboarding's hardware wallet NFC scan from the welcome screen and starts it
    func startHardwareNfcScan() {
        startImportingExistingWallet()
        tapButton(startingWith: "Hardware wallet", until: button(startingWith: "Scan with NFC"))
        tapButton(startingWith: "Scan with NFC", until: buttons["Start NFC Scan"])
        tapButton(named: "Start NFC Scan")
    }

    /// Declines Cloud Backup and accepts the terms after a hardware wallet import, landing on the
    /// wallet screen
    func finishHardwareImport() {
        XCTAssertTrue(staticTexts["Protect this hardware wallet with Cloud Backup?"].waitForExistence(timeout: 30))
        buttons["Not Now"].tap()
        acceptTermsIfNeeded()

        XCTAssertTrue(element("selectedWallet.balance").waitForExistence(timeout: 30))
    }

    /// Opens Receive and returns the address its Copy Address button copies, which the host reads
    /// from the simulator pasteboard
    func copyReceiveAddress(using signer: HardwareSignerControl) throws -> String {
        openReceive()
        return try copyOpenReceiveAddress(using: signer)
    }

    /// Opens Receive and waits until its address has loaded
    func openReceive() {
        tapButton(named: "Receive")

        // Copy Address stays disabled until the address loads
        let copy = buttons["Copy Address"]
        XCTAssertTrue(copy.waitForExistence(timeout: 10))
        waitUntil(copy, matches: NSPredicate(format: "enabled == true"), timeout: 15, "the receive address never loaded")
    }

    /// Copies the address the open Receive sheet shows, which closes the sheet
    func copyOpenReceiveAddress(using signer: HardwareSignerControl) throws -> String {
        let copy = buttons["Copy Address"]
        copy.tap()
        waitUntil(copy, matches: NSPredicate(format: "exists == false"), timeout: 10, "copying the address should close Receive")

        return try signer.simulatorPasteboard().trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// The wallet only looks for new transactions on its own now and then, so pull to refresh
    /// until the funding shows
    func refreshUntilBalance(sats: UInt64) {
        let balance = element("selectedWallet.balance")
        XCTAssertTrue(balance.waitForExistence(timeout: 15))

        let deadline = Date().addingTimeInterval(90)
        while Date() < deadline, satsFromAmountLabel(balance.label) != sats {
            let top = coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.4))
            let bottom = coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.9))
            top.press(forDuration: 0.1, thenDragTo: bottom)
            Thread.sleep(forTimeInterval: 3)
        }

        assertBalance(sats: sats, timeout: 5)
    }

    // MARK: - TAPSIGNER

    /// Scans a TAPSIGNER from Add Wallet in the sidebar; the NFC seam answers from the emulated
    /// card, and the app asks to set it up or import it depending on whether it has a key
    func scanTapSignerFromAddWallet(expectingAlert title: String) {
        tapButton(named: "app.sidebar.open")
        tapButton(named: "Add Wallet")
        scanTapSigner(expectingAlert: title)
    }

    /// Scans the card from Add Wallet and accepts the alert that follows
    func scanTapSigner(expectingAlert title: String) {
        scanNfcFromAddWallet()

        let alert = alerts[title]
        XCTAssertTrue(alert.waitForExistence(timeout: 15), "the TAPSIGNER scan never asked '\(title)'")
        alert.buttons["Yes"].tap()
    }

    /// Starts the NFC scan on the open Add Wallet screen, which the NFC seams answer
    func scanNfcFromAddWallet() {
        tapButton(named: "More wallet import options")
        tapButton(named: "NFC")
    }

    /// Types the CVC into the open TAPSIGNER CVC screen and submits it
    func enterTapSignerCvc(_ cvc: String) {
        let field = secureTextFields["Enter CVC"]
        XCTAssertTrue(field.waitForExistence(timeout: 10))
        field.tap()
        field.typeText(cvc)
        tapButton(named: "Continue")
    }
}
