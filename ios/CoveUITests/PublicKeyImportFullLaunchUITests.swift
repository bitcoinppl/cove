import XCTest

/// Imports a hardware wallet from its bare public key in the real app, scanned through the debug
/// QR seam as a plain QR and as a single-part BBQr, and checks the address type the app infers
/// from the key by the first receive address it shows
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

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        node = FakeNodeControl()
        signer = HardwareSignerControl()
        qr = QrScript()

        // a fresh install on Signet against the fake node
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
        qr.configure(app)
    }

    override func tearDown() {
        qr.remove()
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

        app.openReceive()

        // the path's purpose is the address type, 84' for native and 49' for wrapped segwit
        let derivation = app.staticTexts["Derivation: \(export.firstDerivationPath)"]
        XCTAssertTrue(derivation.waitForExistence(timeout: 10), "Receive should show the first address's path \(export.firstDerivationPath)")

        let address = try app.copyOpenReceiveAddress(using: signer)
        XCTAssertEqual(address, export.firstAddress, "the first receive address doesn't match the key's address type")
    }
}
