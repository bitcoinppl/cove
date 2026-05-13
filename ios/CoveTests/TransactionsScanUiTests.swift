@testable import Cove
import CoveCore
import SwiftUI
import Vision
import XCTest

@MainActor
final class TransactionsScanUiTests: XCTestCase {
    func testEmptyScanningStateShowsProgressCopyWithoutEmptyState() throws {
        let image = try render(
            EmptyWalletScanState(
                scanProgress: WalletScanProgress(
                    phase: .initial,
                    checked: 42,
                    gap: 4,
                    stopGap: 10
                ),
                progressFraction: 0.4
            )
            .frame(width: 393, height: 240)
        )
        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(recognizedText.contains("checking wallet history"), recognizedText)
        XCTAssertTrue(recognizedText.contains("42 addresses checked"), recognizedText)
        XCTAssertFalse(recognizedText.contains("no transactions"), recognizedText)
    }

    func testTransactionsVisibleScanStateOmitsProgressCopy() throws {
        let image = try render(
            VStack(spacing: 12) {
                Text("Preview transaction")
                TransactionsScanProgressStrip(progressFraction: 0.4)
                    .frame(width: 320)
            }
            .frame(width: 393, height: 160)
        )
        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(recognizedText.contains("preview transaction"), recognizedText)
        XCTAssertFalse(recognizedText.contains("checking wallet history"), recognizedText)
        XCTAssertFalse(recognizedText.contains("addresses checked"), recognizedText)
    }

    private func render(_ view: some View) throws -> UIImage {
        let size = CGSize(width: 393, height: 260)
        let hostingController = UIHostingController(rootView: view)
        let window = UIWindow(frame: CGRect(origin: .zero, size: size))
        window.rootViewController = hostingController
        window.makeKeyAndVisible()

        hostingController.view.bounds = window.bounds
        hostingController.view.backgroundColor = .systemBackground
        hostingController.view.setNeedsLayout()
        hostingController.view.layoutIfNeeded()

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

        let recognizedText = request.results?
            .compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: "\n") ?? ""

        let attachment = XCTAttachment(image: image)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)

        return recognizedText
            .lowercased()
            .replacingOccurrences(of: "\n", with: " ")
    }
}
