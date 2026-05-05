@testable import Cove
import CoveCore
import SwiftUI
import Vision
import XCTest

@MainActor
final class OnboardingPromptLayoutTests: XCTestCase {
    func testHardwareCloudBackupPromptRendersFullTitle() throws {
        try assertFixtureContainsHardwareExport()

        let image = try renderHardwareCloudBackupPrompt()
        let attachment = XCTAttachment(image: image)
        attachment.name = "hardware-cloud-backup-prompt"
        attachment.lifetime = .keepAlways
        add(attachment)

        let recognizedText = try recognizedText(in: image)
        let normalizedText = recognizedText
            .lowercased()
            .replacingOccurrences(of: "\n", with: " ")

        XCTAssertTrue(
            normalizedText.contains("protect this hardware wallet with cloud backup"),
            "expected full title in rendered prompt, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            normalizedText.contains("enable cloud backup"),
            "expected primary action in rendered prompt, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            normalizedText.contains("not now"),
            "expected skip action in rendered prompt, got:\n\(recognizedText)"
        )
    }

    private func assertFixtureContainsHardwareExport() throws {
        let fixtureUrl = try XCTUnwrap(
            Bundle(for: Self.self).url(
                forResource: "wallet_2_descriptors",
                withExtension: "txt"
            )
        )
        let format = try FileHandler(filePath: fixtureUrl.path()).read()

        guard case .hardwareExport = format else {
            XCTFail("expected hardware export fixture")
            return
        }
    }

    private func renderHardwareCloudBackupPrompt() throws -> UIImage {
        let size = CGSize(width: 393, height: 852)
        let view = OnboardingHardwareImportCloudBackupChoiceView(onEnable: {}, onSkip: {})
            .frame(width: size.width, height: size.height)

        let hostingController = UIHostingController(rootView: view)
        let window = UIWindow(frame: CGRect(origin: .zero, size: size))
        window.rootViewController = hostingController
        window.makeKeyAndVisible()

        hostingController.view.bounds = window.bounds
        hostingController.view.backgroundColor = .clear
        hostingController.view.setNeedsLayout()
        hostingController.view.layoutIfNeeded()

        let format = UIGraphicsImageRendererFormat()
        format.scale = 3
        let renderer = UIGraphicsImageRenderer(size: size, format: format)

        return renderer.image { _ in
            hostingController.view.drawHierarchy(in: hostingController.view.bounds, afterScreenUpdates: true)
        }
    }

    private func recognizedText(in image: UIImage) throws -> String {
        let cgImage = try XCTUnwrap(image.cgImage)
        let request = VNRecognizeTextRequest()
        request.recognitionLevel = .accurate
        request.usesLanguageCorrection = false

        let handler = VNImageRequestHandler(cgImage: cgImage)
        try handler.perform([request])

        return request.results?
            .compactMap { $0.topCandidates(1).first?.string }
            .joined(separator: "\n") ?? ""
    }
}
