import XCTest

/// Opens QR sheets with the simulator in dark mode and checks which draw on a light background
///
/// Cheap scanner cameras, such as SeedSigner's, fail to find a QR code drawn on a dark sheet, so a
/// sheet that draws its code straight onto its background, such as the xpub export, stays light
/// whatever the appearance. Receive draws its address on a white card with a four-module quiet
/// zone, which `KeyTeleportPacketQrTests` measures, so it follows the app into dark mode. The test
/// reads the screen beside the QR code, where the sheet's own background shows. Needs the fake node
/// and the hardware signer host, which `xtask ios-ui` (`just ios-ui-background`) starts for every
/// run
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

    func testExportStaysLightAndReceiveFollowsDarkMode() throws {
        try signer.setSimulatorAppearance(dark: true)
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()

        app.openReceive()
        assertSheetBackground("Receive", at: Self.receiveBackground, reads: \.isDark, as: "a dark background")
        app.tapButton(named: "Done")
        XCTAssertTrue(app.buttons["Copy Address"].waitForNonExistence(timeout: 10), "Receive never closed")

        openXpubQrExport()
        assertSheetBackground("Export Xpub", at: Self.exportBackground, reads: \.isLight, as: "a light background")
    }

    // MARK: - Steps

    private func openXpubQrExport() {
        app.tapButton(named: "selectedWallet.more")
        app.tapButton(named: "Export Xpub")
        app.tapButton(named: "QR Code")
        XCTAssertTrue(app.buttons["Decrease density"].waitForExistence(timeout: 15), "the xpub QR code never showed")
    }

    /// Waits for the open sheet's background to pass `expected` at every point, which it does once
    /// the sheet has finished sliding up
    private func assertSheetBackground(
        _ sheet: String,
        at points: [CGPoint],
        reads expected: (ScreenColor) -> Bool,
        as description: String
    ) {
        let deadline = Date().addingTimeInterval(5)
        var shown = points.map(ScreenColor.read(at:))
        while !shown.allSatisfy(expected), Date() < deadline {
            Thread.sleep(forTimeInterval: 0.25)
            shown = points.map(ScreenColor.read(at:))
        }

        // attached on success too, so a kept result bundle shows each sheet in dark mode
        let screenshot = XCTAttachment(screenshot: XCUIScreen.main.screenshot())
        screenshot.name = "\(sheet) in dark mode"
        screenshot.lifetime = .keepAlways
        add(screenshot)

        for (point, color) in zip(points, shown) {
            XCTAssertTrue(expected(color), "\(sheet) draws \(color) at \(point) in dark mode, not \(description)")
        }
    }
}
