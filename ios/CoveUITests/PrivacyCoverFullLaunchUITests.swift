import XCTest

/// Checks the privacy cover goes over a sheet a screen presents whenever the app goes inactive, and
/// comes down once the app is back
///
/// With no app lock set up nothing dismisses the sheet on the way out, so the cover is all that
/// keeps it out of the app switcher. A cover presented as a `.fullScreenCover` waited behind a sheet
/// a screen presented, such as Receive, and never showed. Pulling Notification Center down makes
/// the app inactive, and hides the app from the screen while it is up, so the test checks the cover
/// is in the tree and spans the app's window then. Once the app is back it also reads the screen,
/// since a cover fading out stays on it after it leaves the tree, and taps Copy Address, since a
/// button XCUITest calls hittable can still sit under a layer that takes the tap. The screen check
/// needs Receive lighter than the cover's black, so the test switches the simulator to light mode,
/// which other tests leave dark. Needs the fake node and the hardware signer host, which
/// `xtask ios-ui` (`just ios-ui-background`) starts for every run
final class PrivacyCoverFullLaunchUITests: XCTestCase {
    private static let esploraUrl = UITestHostPorts.esploraUrl

    /// Points near the edges of the Receive sheet, clear of its QR code and its card, where the
    /// sheet's own background shows
    private static let sheetBackground = [
        CGPoint(x: 0.03, y: 0.5),
        CGPoint(x: 0.97, y: 0.5),
        CGPoint(x: 0.5, y: 0.97),
    ]

    private var app: XCUIApplication!
    private var springboard: XCUIApplication!
    private var signer: HardwareSignerControl!

    override func setUp() {
        super.setUp()
        continueAfterFailure = false

        app = XCUIApplication()
        springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
        signer = HardwareSignerControl()
        app.launchEnvironment["COVE_UI_TEST_ESPLORA_URL"] = Self.esploraUrl
    }

    /// Receive, which the wallet screen presents, is covered while Notification Center is down, and
    /// shows again with nothing over it once the app is back
    func testThePrivacyCoverGoesOverReceiveWhileNotificationCenterIsDown() throws {
        try signer.setSimulatorAppearance(dark: false)
        app.launch(expecting: app.staticTexts["Welcome to Cove"])
        app.importTestMnemonic()
        app.openReceive()

        let copy = app.buttons["Copy Address"]
        let cover = app.otherElements["privacy.cover"]
        XCTAssertTrue(copy.waitForExistence(timeout: 10), "Receive never came up")
        XCTAssertFalse(cover.exists, "the privacy cover was up before leaving the app")

        // read before the cover's window exists, so the frame is the app's own window's
        let appFrame = app.windows.firstMatch.frame
        let uncovered = Self.sheetBackground.map(screenColor(at:))
        for (point, color) in zip(Self.sheetBackground, uncovered) {
            XCTAssertFalse(color.isBlack, "Receive is black at \(point), so the screen can't tell it from the cover")
        }

        pullDownNotificationCenter()
        XCTAssertTrue(cover.waitForExistence(timeout: 5), "the privacy cover never covered Receive")
        XCTAssertEqual(cover.frame, appFrame, "the privacy cover left part of the app uncovered")

        closeNotificationCenter()
        XCTAssertTrue(cover.waitForNonExistence(timeout: 5), "the privacy cover stayed up after returning")
        assertScreen(shows: uncovered)

        // Copy Address closes Receive, so only a tap that lands closes it
        copy.tap()
        XCTAssertTrue(copy.waitForNonExistence(timeout: 10), "something stayed up over Receive and took the tap")
    }

    // MARK: - Steps

    /// Waits for the screen to show the given colors at the sheet's background points again, which it
    /// does once the cover has faded out
    private func assertScreen(shows expected: [ScreenColor]) {
        let deadline = Date().addingTimeInterval(5)
        var shown = Self.sheetBackground.map(screenColor(at:))
        while !zip(shown, expected).allSatisfy({ $0.matches($1) }), Date() < deadline {
            Thread.sleep(forTimeInterval: 0.25)
            shown = Self.sheetBackground.map(screenColor(at:))
        }

        for (point, (color, wanted)) in zip(Self.sheetBackground, zip(shown, expected)) {
            XCTAssertTrue(color.matches(wanted), "the screen shows \(color) at \(point) after returning, not Receive's \(wanted)")
        }
    }

    /// Pulls Notification Center down from the top edge, which makes the app inactive
    private func pullDownNotificationCenter() {
        let topEdge = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.25, dy: 0.002))
        let middle = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.25, dy: 0.7))
        topEdge.press(forDuration: 0.05, thenDragTo: middle)
    }

    /// Pushes Notification Center back up from the bottom edge and waits for the app to come back
    private func closeNotificationCenter() {
        let bottomEdge = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.995))
        let upper = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.2))
        bottomEdge.press(forDuration: 0.05, thenDragTo: upper)
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 10), "the app never came back")
    }

    /// The screen's color at a point given as a fraction of its size, read from a fresh screenshot
    private func screenColor(at point: CGPoint) -> ScreenColor {
        let image = XCUIScreen.main.screenshot().image.cgImage
        let pixel = image.flatMap { pixel(of: $0, at: point) }
        XCTAssertNotNil(pixel, "the screenshot could not be read at \(point)")
        return pixel ?? ScreenColor(red: 0, green: 0, blue: 0)
    }

    private func pixel(of image: CGImage, at point: CGPoint) -> ScreenColor? {
        let x = Int(CGFloat(image.width) * point.x)
        let y = Int(CGFloat(image.height) * point.y)
        var pixel = [UInt8](repeating: 0, count: 4)

        // draw the image shifted so the pixel lands in a one-pixel context, whose origin is the
        // bottom left
        let drawn = pixel.withUnsafeMutableBytes { bytes -> Bool in
            guard let context = CGContext(
                data: bytes.baseAddress,
                width: 1,
                height: 1,
                bitsPerComponent: 8,
                bytesPerRow: 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            ) else { return false }

            let origin = CGPoint(x: -x, y: -(image.height - 1 - y))
            context.draw(image, in: CGRect(origin: origin, size: CGSize(width: image.width, height: image.height)))
            return true
        }

        guard drawn else { return nil }
        return ScreenColor(red: pixel[0], green: pixel[1], blue: pixel[2])
    }
}

/// A screen pixel's color
private struct ScreenColor: CustomStringConvertible {
    private static let tolerance = 8

    let red: UInt8
    let green: UInt8
    let blue: UInt8

    /// Black, as the privacy cover draws it
    var isBlack: Bool { max(red, green, blue) < 32 }

    var description: String { "rgb(\(red), \(green), \(blue))" }

    /// Whether this is the same color as `other`, within the small drift screenshots of the same
    /// picture show
    func matches(_ other: ScreenColor) -> Bool {
        abs(Int(red) - Int(other.red)) <= Self.tolerance
            && abs(Int(green) - Int(other.green)) <= Self.tolerance
            && abs(Int(blue) - Int(other.blue)) <= Self.tolerance
    }
}
