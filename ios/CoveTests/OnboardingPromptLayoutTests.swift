@testable import Cove
import CoveCore
import SwiftUI
import Vision
import XCTest

@MainActor
final class OnboardingPromptLayoutTests: XCTestCase {
    func testCloudDiscoveryDecisionRowsRenderRequiredActions() throws {
        let checking = try recognizedText(in: render(
            CloudCheckContent(onContinue: {})
        ))
        XCTAssertTrue(checking.contains("Looking for iCloud backup"))
        XCTAssertTrue(checking.contains("Continue Setup"))

        let unavailable = try recognizedText(in: render(
            OnboardingRestoreUnavailableScreen(
                onCheckAgain: {},
                onContinue: {},
                onBack: {}
            )
        ))
        XCTAssertTrue(unavailable.contains("Nothing visible yet"))
        XCTAssertTrue(unavailable.contains("Check Again"))
        XCTAssertTrue(unavailable.contains("Continue Setup"))

        let bitcoinChoice = try recognizedText(in: render(
            OnboardingBitcoinChoiceScreen(
                errorMessage: nil,
                onRestoreFromCoveBackup: {},
                onNewHere: {},
                onHasBitcoin: {}
            )
        ))
        XCTAssertTrue(bitcoinChoice.contains("Restore from Cove backup"))

        let storageChoice = try recognizedText(in: render(
            OnboardingStorageChoiceScreen(
                errorMessage: nil,
                onRestoreFromCoveBackup: {},
                onSelectStorage: { _ in },
                onBack: {}
            )
        ))
        XCTAssertTrue(storageChoice.contains("Restore from Cove backup"))
    }

    func testCloudRestoreOfferProjectsProviderHint() throws {
        let text = try recognizedText(in: render(
            CloudRestoreOfferView(
                onRestore: {},
                onSkip: {},
                providerHint: CloudRestoreProviderHint(
                    providerName: "Apple Passwords",
                    registeredAt: 1_777_612_800,
                    nameSuffix: "09IX"
                )
            )
        ))

        let normalizedText = text.replacingOccurrences(of: "\n", with: " ")

        XCTAssertTrue(normalizedText.contains("Cove Cloud Backup (09IX)"), "expected passkey suffix, got:\n\(text)")
        XCTAssertTrue(normalizedText.contains("Provider Details"), "expected provider details, got:\n\(text)")
        XCTAssertTrue(normalizedText.contains("Apple Passwords"), "expected provider name, got:\n\(text)")
        XCTAssertTrue(
            normalizedText.contains("Your passkey is stored securely by Apple Passwords"),
            "expected provider-specific storage copy, got:\n\(text)"
        )
    }

    func testSoftwareImportProjectsLateCloudRestoreOffer() {
        let view = OnboardingSoftwareImportFlowView(
            errorMessage: nil,
            cloudRestoreAlertVisible: .constant(true),
            onImported: { _ in },
            onCreateWallet: {},
            onRestoreFromCloudBackup: {},
            onDismissCloudRestoreAlert: {},
            onBack: {}
        )
        let alert = presentedAlert(in: view)

        XCTAssertEqual(alert?.title, "Cove backup found")
        XCTAssertEqual(alert?.actions.compactMap(\.title), ["Restore from Cove backup", "Continue setup"])
    }

    func testHardwareImportProjectsLateCloudRestoreOffer() {
        let view = OnboardingHardwareImportFlowView(
            cloudRestoreAlertVisible: .constant(true),
            onImported: { _ in },
            onRestoreFromCloudBackup: {},
            onDismissCloudRestoreAlert: {},
            onBack: {}
        )
        let alert = presentedAlert(in: view)

        XCTAssertEqual(alert?.title, "Cove backup found")
        XCTAssertEqual(alert?.actions.compactMap(\.title), ["Restore from Cove backup", "Continue setup"])
    }

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
        try render(OnboardingHardwareImportCloudBackupChoiceView(onEnable: {}, onSkip: {}))
    }

    private func render(_ content: some View) throws -> UIImage {
        let size = CGSize(width: 393, height: 852)
        let view = content
            .frame(width: size.width, height: size.height)

        let hostingController = UIHostingController(rootView: view)
        let window = testWindow(size: size)
        window.rootViewController = hostingController
        window.makeKeyAndVisible()

        hostingController.view.bounds = window.bounds
        hostingController.view.backgroundColor = .clear
        hostingController.view.setNeedsLayout()
        hostingController.view.layoutIfNeeded()
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))

        let format = UIGraphicsImageRendererFormat()
        format.scale = 3
        let renderer = UIGraphicsImageRenderer(size: size, format: format)

        return renderer.image { _ in
            window.drawHierarchy(in: window.bounds, afterScreenUpdates: true)
        }
    }

    private func presentedAlert(in content: some View) -> UIAlertController? {
        let size = CGSize(width: 393, height: 852)
        let hostingController = UIHostingController(rootView: content)
        let window = testWindow(size: size)
        window.rootViewController = hostingController
        window.makeKeyAndVisible()
        RunLoop.main.run(until: Date().addingTimeInterval(0.1))

        return hostingController.presentedViewController as? UIAlertController
    }

    private func testWindow(size: CGSize) -> UIWindow {
        if let scene = UIApplication.shared.connectedScenes.compactMap({ $0 as? UIWindowScene }).first {
            let window = UIWindow(windowScene: scene)
            window.frame = CGRect(origin: .zero, size: size)
            return window
        }

        return UIWindow(frame: CGRect(origin: .zero, size: size))
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
