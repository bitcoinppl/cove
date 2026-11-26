@testable import Cove
import SwiftUI
import XCTest

/// KeyTeleport's packet QR must keep the four-module white quiet zone that QR scanners look for,
/// since it shows on a dark screen and scanners such as the Coldcard Q's find no code without it
@MainActor
final class KeyTeleportPacketQrTests: XCTestCase {
    /// The receiver request from the Rust KeyTeleport tests
    private static let receiverPacket = "B$2R0100VHT2AGUUH7KUZUUSTOWOIWHJX3XM7GA2N4BHQOXDFHXLVHVA7K6ZO"

    /// The quiet zone, in modules, that the QR code standard asks for
    private static let quietZoneModules: CGFloat = 4

    func testPacketQrKeepsAFourModuleQuietZone() throws {
        let renderer = ImageRenderer(content: KeyTeleportPacketQr(text: Self.receiverPacket))
        renderer.proposedSize = ProposedViewSize(width: 280, height: nil)
        renderer.scale = 3

        let image = try XCTUnwrap(renderer.cgImage, "the packet QR didn't render")
        let pixels = try Pixels(image)

        // the top row of the two upper finder patterns, each 7 modules wide. The renderer leaves
        // the card's surroundings transparent, so the white quiet zone ends where they begin
        let top = try XCTUnwrap((0 ..< pixels.height).first { y in (0 ..< pixels.width).contains { pixels.isDark($0, y) } })
        let left = try XCTUnwrap((0 ..< pixels.width).first { pixels.isDark($0, top) })
        let right = try XCTUnwrap((0 ..< pixels.width).last { pixels.isDark($0, top) })
        let finderWidth = (left ..< pixels.width).prefix { pixels.isDark($0, top) }.count
        let module = CGFloat(finderWidth) / 7
        let finderMiddle = left + finderWidth / 2

        // white pixels between each edge of the code and the card's edge
        let quietZones = [
            ("top", (0 ..< top).reversed().prefix { pixels.isWhite(finderMiddle, $0) }.count),
            ("left", (0 ..< left).reversed().prefix { pixels.isWhite($0, top) }.count),
            ("right", (right + 1 ..< pixels.width).prefix { pixels.isWhite($0, top) }.count),
        ]

        for (side, pixelsWide) in quietZones {
            let modules = CGFloat(pixelsWide) / module
            XCTAssertGreaterThanOrEqual(
                modules,
                Self.quietZoneModules,
                "the QR's \(side) quiet zone is \(modules) modules"
            )
        }
    }
}

private struct PixelReadFailed: Error {}

/// An image's pixels, read back as RGBA
private struct Pixels {
    let width: Int
    let height: Int
    private let bytes: [UInt8]

    init(_ image: CGImage) throws {
        width = image.width
        height = image.height
        var bytes = [UInt8](repeating: 0, count: width * height * 4)
        let drawn = bytes.withUnsafeMutableBytes { buffer in
            let context = CGContext(
                data: buffer.baseAddress,
                width: image.width,
                height: image.height,
                bitsPerComponent: 8,
                bytesPerRow: image.width * 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )
            context?.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
            return context != nil
        }

        guard drawn else { throw PixelReadFailed() }
        self.bytes = bytes
    }

    /// Whether the pixel, counted from the top left, is an opaque dark module
    func isDark(_ x: Int, _ y: Int) -> Bool {
        let offset = (y * width + x) * 4
        let alpha = Int(bytes[offset + 3])
        let sum = Int(bytes[offset]) + Int(bytes[offset + 1]) + Int(bytes[offset + 2])
        return alpha > 128 && sum < 384
    }

    /// Whether the pixel, counted from the top left, is opaque white
    func isWhite(_ x: Int, _ y: Int) -> Bool {
        let offset = (y * width + x) * 4
        let channels = bytes[offset ..< offset + 4]
        return channels.allSatisfy { $0 > 204 }
    }
}
