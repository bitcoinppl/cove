import XCTest

/// A screen pixel's color, read from a screenshot
struct ScreenColor: CustomStringConvertible {
    private static let tolerance = 8

    let red: UInt8
    let green: UInt8
    let blue: UInt8

    /// Black, as the privacy cover draws it
    var isBlack: Bool { max(red, green, blue) < 32 }

    /// Near white, as a light sheet's background is
    var isLight: Bool { min(red, green, blue) > 220 }

    /// Dark, as a sheet's background is in dark mode
    var isDark: Bool { max(red, green, blue) < 96 }

    var description: String { "rgb(\(red), \(green), \(blue))" }

    /// Whether this is the same color as `other`, within the small drift screenshots of the same
    /// picture show
    func matches(_ other: ScreenColor) -> Bool {
        abs(Int(red) - Int(other.red)) <= Self.tolerance
            && abs(Int(green) - Int(other.green)) <= Self.tolerance
            && abs(Int(blue) - Int(other.blue)) <= Self.tolerance
    }

    /// The screen's color at a point given as a fraction of its size, read from a fresh screenshot
    static func read(at point: CGPoint) -> ScreenColor {
        let image = XCUIScreen.main.screenshot().image.cgImage
        let color = image.flatMap { pixelColor(of: $0, at: point) }
        XCTAssertNotNil(color, "the screenshot could not be read at \(point)")
        return color ?? ScreenColor(red: 0, green: 0, blue: 0)
    }

    private static func pixelColor(of image: CGImage, at point: CGPoint) -> ScreenColor? {
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
