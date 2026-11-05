import XCTest

/// Imports a hardware wallet from its bare public key in the real app, scanned through the debug
/// QR seam as a plain QR and as a single-part BBQr, or read through the debug NFC seam from a tag's
/// text record, and checks the address type the app infers from the key by the first receive
/// address it shows
///
/// The signer host derives the key and the expected address from the test mnemonic, the address
/// straight from the key at `/0/0` rather than from a descriptor. The fake node is reset first, so
/// the account has no history and Receive opens at index 0. Needs the fake node and the hardware
/// signer host, which `xtask ios-ui` (`just ios-ui-background`) starts for every run
final class PublicKeyImportFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    private var app: XCUIApplication!
    private var node: FakeNodeControl!
    private var signer: HardwareSignerControl!
    private var qr: QrScript!
    private var nfc: NfcNdefScript!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        signer = HardwareSignerControl()
        qr = QrScript()
        nfc = NfcNdefScript()

        // a fresh install on Signet against the fake node
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        qr.configure(app)
        nfc.configure(app)
    }

    override func tearDown() {
        qr.remove()
        nfc.remove()
        super.tearDown()
    }

    /// A vpub names a BIP84 account, so it imports as native segwit
    func testPlainVpubQrImportsNativeSegwitWithItsFirstAddress() throws {
        let export = try publicKeyExport(.nativeSegwit)
        XCTAssertTrue(export.key.hasPrefix("vpub"))

        try importAndCheckFirstAddress(export, frames: [export.key])
    }

    /// A upub names a BIP49 account, so it imports as wrapped segwit, not the native segwit default
    func testSingleBbqrUpubImportsWrappedSegwitWithItsFirstAddress() throws {
        let export = try publicKeyExport(.wrappedSegwit)
        XCTAssertTrue(export.key.hasPrefix("upub"))

        // a BBQr header is B$, the encoding and file type, then the part count and index
        XCTAssertEqual(String(export.bbqr.prefix(8).suffix(4)), "0100", "the export must be a single BBQr part")

        try importAndCheckFirstAddress(export, frames: [export.bbqr])
    }

    /// A vpub in a tag's text record imports from onboarding's NFC scan, where a new user first
    /// holds a tag to the phone. The record's status byte and language code must not reach the key
    func testNfcTextRecordVpubImportsFromOnboarding() throws {
        let export = try publicKeyExport(.nativeSegwit)
        XCTAssertTrue(export.key.hasPrefix("vpub"))

        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.startHardwareNfcScan()
        try nfc.showTextRecord(export.key)
        app.finishHardwareImport()

        try checkFirstAddress(export)
    }

    /// A upub in a tag's text record imports from Add Wallet's NFC scan as wrapped segwit
    func testNfcTextRecordUpubImportsFromAddWallet() throws {
        let first = try publicKeyExport(.nativeSegwit)
        let export = try signer.publicKeyExport(.wrappedSegwit)
        XCTAssertTrue(export.key.hasPrefix("upub"))

        // Add Wallet is in the sidebar once the app has a wallet
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.openHardwareExportScanner()
        try qr.show([first.key])
        app.finishHardwareImport()

        app.tapButton(named: "app.sidebar.open")
        app.tapButton(named: "Add Wallet")
        app.scanNfcFromAddWallet()
        try nfc.showTextRecord(export.key)

        let imported = app.alerts["Success"]
        XCTAssertTrue(imported.waitForExistence(timeout: 30), "the NFC import never reported success")
        imported.buttons["OK"].tap()
        XCTAssertTrue(app.element("selectedWallet.balance").waitForExistence(timeout: 30))

        try checkFirstAddress(export)
    }

    private func publicKeyExport(_ addressType: HardwareSignerControl.AddressType) throws -> HardwareSignerControl.PublicKeyExport {
        try node.requireHealthy()
        try signer.requireHealthy()
        try node.reset()

        return try signer.publicKeyExport(addressType)
    }

    private func importAndCheckFirstAddress(_ export: HardwareSignerControl.PublicKeyExport, frames: [String]) throws {
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.openHardwareExportScanner()
        try qr.show(frames)
        app.finishHardwareImport()

        try checkFirstAddress(export)
    }

    private func checkFirstAddress(_ export: HardwareSignerControl.PublicKeyExport) throws {
        app.openReceive()

        // the path's purpose is the address type, 84' for native and 49' for wrapped segwit
        let derivation = app.staticTexts["Derivation: \(export.firstDerivationPath)"]
        XCTAssertTrue(derivation.waitForExistence(timeout: 10), "Receive should show the first address's path \(export.firstDerivationPath)")

        let address = try app.copyOpenReceiveAddress(using: signer)
        XCTAssertEqual(address, export.firstAddress, "the first receive address doesn't match the key's address type")
    }
}
