@testable import Cove
import CoveCore
import SwiftUI
import Vision
import XCTest

@MainActor
final class MoreInfoPopoverTests: XCTestCase {
    func testInitialScanWithoutTransactionsHidesCoinControlAndTransactionExport() async throws {
        try await bootstrapIfNeeded()

        let manager = WalletManager(preview: .only)
        manager.loadState = .scanning([])

        let recognizedText = try recognizedText(in: manager)

        XCTAssertTrue(recognizedText.contains("wallet settings"), recognizedText)
        XCTAssertFalse(recognizedText.contains("manage utxos"), recognizedText)
        XCTAssertFalse(recognizedText.contains("export transactions"), recognizedText)
    }

    func testWalletWithTransactionsOffersCoinControl() async throws {
        try await bootstrapIfNeeded()

        let manager = WalletManager(preview: .only)
        manager.loadState = .loaded([transactionPreviewConfirmedNew()])

        let recognizedText = try recognizedText(in: manager)

        XCTAssertTrue(recognizedText.contains("manage utxos"), recognizedText)
        XCTAssertTrue(recognizedText.contains("export transactions"), recognizedText)
    }

    private func bootstrapIfNeeded() async throws {
        do {
            _ = try await bootstrap()
        } catch AppInitError.AlreadyCalled(_) {}

        // the test host app bootstraps on its own, so AlreadyCalled can return before it finishes
        let deadline = Date().addingTimeInterval(30)
        while bootstrapProgress() != .complete {
            try XCTUnwrap(Date() < deadline ? true : nil, "bootstrap did not complete")
            try await Task.sleep(for: .milliseconds(50))
        }
    }

    private func recognizedText(in manager: WalletManager) throws -> String {
        let popover = MoreInfoPopover(
            manager: manager,
            importLabels: {},
            exportLabels: {},
            exportXpub: {}
        )
        .environment(AppManager.shared)
        .frame(width: 393, height: 420)

        let image = try render(popover)
        let cgImage = try XCTUnwrap(image.cgImage)
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false
        try VNImageRequestHandler(cgImage: cgImage).perform([request])

        let attachment = XCTAttachment(image: image)
        attachment.lifetime = .keepAlways
        add(attachment)

        return (request.results ?? [])
            .compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: " ")
            .lowercased()
    }

    private func render(_ view: some View) throws -> UIImage {
        let size = CGSize(width: 393, height: 420)
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

        return UIGraphicsImageRenderer(size: size, format: format).image { _ in
            hostingController.view.drawHierarchy(in: hostingController.view.bounds, afterScreenUpdates: true)
        }
    }
}
