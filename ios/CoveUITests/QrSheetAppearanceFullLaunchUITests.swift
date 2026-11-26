import XCTest

/// Opens QR sheets with the simulator in dark mode and checks each draws on a light background
///
/// Cheap scanner cameras, such as SeedSigner's, fail to find a QR code drawn on a dark sheet, so
/// every sheet that shows a QR code for another device stays light whatever the appearance. The
/// test reads the screen beside the QR code, where the sheet's own background shows. Needs the fake
/// node and the hardware signer host, which `xtask ios-ui` (`just ios-ui-background`) starts for
/// every run
final class QrSheetAppearanceFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    /// Points in Receive's side margins, outside its address card, and at its bottom edge, where
    /// the sheet's background shows
    private static let receiveBackground = [
        CGPoint(x: 0.03, y: 0.75),
        CGPoint(x: 0.97, y: 0.75),
        CGPoint(x: 0.5, y: 0.97),
    ]

    /// Points in the export sheet's side margins beside the QR code. A sheet below full height
    /// floats about 2% in from the screen's edges, so these sit further in than Receive's
    private static let exportBackground = [
        CGPoint(x: 0.05, y: 0.75),
        CGPoint(x: 0.95, y: 0.75),
        CGPoint(x: 0.05, y: 0.9),
    ]

    private var app: XCUIApplication!
    private var signer: HardwareSignerControl!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        signer = HardwareSignerControl()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
    }

    func testQrSheetsStayLightInDarkMode() throws {
        try signer.setSimulatorAppearance(dark: true)
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()

        app.openReceive()
        assertSheetIsLight("Receive", at: Self.receiveBackground)
        app.tapButton(named: "Done")
        XCTAssertTrue(app.buttons["Copy Address"].waitForNonExistence(timeout: 10), "Receive never closed")

        openXpubQrExport()
        assertSheetIsLight("Export Xpub", at: Self.exportBackground)
    }

    // MARK: - Steps

    private func openXpubQrExport() {
        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Export Xpub")
        app.tapButton(named: "QR Code")
        XCTAssertTrue(app.buttons["Decrease density"].waitForExistence(timeout: 15), "the xpub QR code never showed")
    }

    /// Waits for the open sheet's background to read light at every point, which it does once the
    /// sheet has finished sliding up
    private func assertSheetIsLight(_ sheet: String, at points: [CGPoint]) {
        let deadline = Date().addingTimeInterval(5)
        var shown = points.map(ScreenColor.read(at:))
        while !shown.allSatisfy(\.isLight), Date() < deadline {
            Thread.sleep(forTimeInterval: 0.25)
            shown = points.map(ScreenColor.read(at:))
        }

        for (point, color) in zip(points, shown) {
            XCTAssertTrue(color.isLight, "\(sheet) draws \(color) at \(point) in dark mode, not a light background")
        }
    }
}
