@testable import Cove
import CoveCore
import SwiftUI
import Vision
import XCTest

@MainActor
final class HotWalletCreateScreenLayoutTests: XCTestCase {
    func testCompactRecoveryWordsLayoutCanScrollToPrimaryAction() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let manager = PendingWalletManager(numberOfWords: .twelve)
        let view = NavigationStack {
            WordsView(manager: manager)
                .environment(AppManager.shared)
        }
        .frame(width: size.width, height: size.height)

        let hostingController = UIHostingController(rootView: view)
        let window = UIWindow(frame: CGRect(origin: .zero, size: size))
        window.rootViewController = hostingController
        window.makeKeyAndVisible()

        hostingController.view.bounds = window.bounds
        hostingController.view.backgroundColor = .clear
        hostingController.view.setNeedsLayout()
        hostingController.view.layoutIfNeeded()

        if screenshotMode() == "initial" {
            let image = render(hostingController: hostingController, size: size)
            addScreenshotAttachment(image, name: "compact-recovery-words-initial")
            try saveScreenshotIfRequested(image)
            return
        }

        let scrollView = try XCTUnwrap(findVerticallyScrollableView(in: hostingController.view))
        let maxOffsetY = max(
            scrollView.contentSize.height - scrollView.bounds.height + scrollView.adjustedContentInset.bottom,
            -scrollView.adjustedContentInset.top
        )
        scrollView.setContentOffset(
            CGPoint(x: 0, y: maxOffsetY),
            animated: false
        )
        hostingController.view.layoutIfNeeded()

        let image = render(hostingController: hostingController, size: size)
        addScreenshotAttachment(image, name: "compact-recovery-words-after-scroll")
        try saveScreenshotIfRequested(image)
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("save wallet"),
            "expected compact recovery words screen to scroll to Save Wallet, got:\n\(recognizedText)"
        )
    }

    private func bootstrapIfNeeded() async throws {
        do {
            _ = try await bootstrap()
        } catch AppInitError.AlreadyCalled(_) {}
    }

    private func findVerticallyScrollableView(in view: UIView) -> UIScrollView? {
        if let scrollView = view as? UIScrollView,
           scrollView.contentSize.height > scrollView.bounds.height + 1
        {
            return scrollView
        }

        for subview in view.subviews {
            if let scrollView = findVerticallyScrollableView(in: subview) {
                return scrollView
            }
        }

        return nil
    }

    private func render(
        hostingController: UIHostingController<some View>,
        size: CGSize
    ) -> UIImage {
        let format = UIGraphicsImageRendererFormat()
        format.scale = 3
        let renderer = UIGraphicsImageRenderer(size: size, format: format)

        return renderer.image { _ in
            hostingController.view.drawHierarchy(in: hostingController.view.bounds, afterScreenUpdates: true)
        }
    }

    private func normalizedRecognizedText(in image: UIImage) throws -> String {
        let cgImage = try XCTUnwrap(image.cgImage)
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false

        let handler = VNImageRequestHandler(cgImage: cgImage)
        try handler.perform([request])

        return request.results?
            .compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: "\n")
            .lowercased()
            .replacingOccurrences(of: "\n", with: " ") ?? ""
    }

    private func assertPrimaryActionIsNotClippedAtBottom(
        in image: UIImage,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let cgImage = try XCTUnwrap(image.cgImage)
        let sampleHeight = min(16, cgImage.height)
        let bottomRect = CGRect(
            x: 0,
            y: cgImage.height - sampleHeight,
            width: cgImage.width,
            height: sampleHeight
        )
        let bottomEdge = try XCTUnwrap(cgImage.cropping(to: bottomRect))
        let pixels = try rgbaPixels(in: bottomEdge)
        let brightPixelCount = stride(from: 0, to: pixels.count, by: 4).count(where: { offset in
            pixels[offset] > 210 &&
                pixels[offset + 1] > 210 &&
                pixels[offset + 2] > 210
        })
        let maximumAllowedBrightPixels = max(1, bottomEdge.width * bottomEdge.height / 20)

        XCTAssertLessThan(
            brightPixelCount,
            maximumAllowedBrightPixels,
            "expected dark padding below the primary action; bright pixels at the bottom edge indicate the button is clipped",
            file: file,
            line: line
        )
    }

    private func rgbaPixels(in image: CGImage) throws -> [UInt8] {
        let bytesPerPixel = 4
        let bytesPerRow = image.width * bytesPerPixel
        var pixels = [UInt8](repeating: 0, count: image.height * bytesPerRow)
        let colorSpace = CGColorSpaceCreateDeviceRGB()
        let bitmapInfo = CGImageAlphaInfo.premultipliedLast.rawValue
        let context = try XCTUnwrap(
            CGContext(
                data: &pixels,
                width: image.width,
                height: image.height,
                bitsPerComponent: 8,
                bytesPerRow: bytesPerRow,
                space: colorSpace,
                bitmapInfo: bitmapInfo
            )
        )

        context.draw(
            image,
            in: CGRect(x: 0, y: 0, width: image.width, height: image.height)
        )

        return pixels
    }

    private func addScreenshotAttachment(_ image: UIImage, name: String) {
        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }

    private func screenshotMode() -> String? {
        screenshotEnvironmentValue("COVE_LAYOUT_SCREENSHOT_MODE")
    }

    private func saveScreenshotIfRequested(_ image: UIImage) throws {
        guard let name = screenshotEnvironmentValue("COVE_LAYOUT_SCREENSHOT_NAME") else {
            return
        }

        let documentsDirectory = try FileManager.default.url(
            for: .documentDirectory,
            in: .userDomainMask,
            appropriateFor: nil,
            create: true
        )
        let screenshotDirectory = documentsDirectory.appendingPathComponent("layout-screenshots")
        try FileManager.default.createDirectory(at: screenshotDirectory, withIntermediateDirectories: true)

        let screenshotUrl = screenshotDirectory.appendingPathComponent(name)
        try XCTUnwrap(image.pngData()).write(to: screenshotUrl)
    }

    private func screenshotEnvironmentValue(_ key: String) -> String? {
        let environment = ProcessInfo.processInfo.environment

        return environment[key] ?? environment["TEST_RUNNER_\(key)"]
    }
}
