@testable import Cove
import CoveCore
import SwiftUI
import Vision
import XCTest

@MainActor
final class HotWalletCreateScreenLayoutTests: XCTestCase {
    func testCompactNewWalletSelectActionsAreVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: NavigationStack { NewWalletSelectScreen() }
                .environment(AppManager.shared)
                .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-new-wallet-select")
        try saveAuditScreenshotIfDirectoryRequested(image, name: "new-wallet-select-after.png")
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("hardware wallet"),
            "expected compact new wallet select screen to show Hardware Wallet, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            recognizedText.contains("on this device"),
            "expected compact new wallet select screen to show On This Device, got:\n\(recognizedText)"
        )
    }

    func testCompactHotWalletSelectActionsAreVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: NavigationStack { HotWalletSelectScreen() }
                .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-hot-wallet-select")
        try saveAuditScreenshotIfDirectoryRequested(image, name: "hot-wallet-select-after.png")
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("create new wallet"),
            "expected compact hot wallet select screen to show Create new wallet, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            recognizedText.contains("import existing wallet"),
            "expected compact hot wallet select screen to show Import existing wallet, got:\n\(recognizedText)"
        )
    }

    func testCompactVerificationCompleteActionIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: NavigationStack {
                VerificationCompleteScreen(
                    manager: WalletManager(preview: "preview_only"),
                    onVerified: {}
                )
            }
            .environment(AppManager.shared)
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-verification-complete")
        try saveAuditScreenshotIfDirectoryRequested(image, name: "verification-complete-after.png")
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("go to wallet"),
            "expected compact verification complete screen to show Go To Wallet, got:\n\(recognizedText)"
        )
    }

    func testCompactTapSignerSetupSuccessActionIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: TapSignerContainer(
                route: .setupSuccess(
                    tapSignerPreviewNew(preview: true),
                    tapSignerSetupCompleteNew(preview: true)
                )
            )
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-tap-signer-setup-success")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "tap-signer-setup-success")
        try assertLightScreenBottomEdgeIsClear(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("continue"),
            "expected compact TapSigner setup success screen to show Continue, got:\n\(recognizedText)"
        )
    }

    func testCompactTapSignerImportSuccessActionIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let setup = tapSignerSetupCompleteNew(preview: true)
        let image = render(
            view: TapSignerContainer(
                route: .importSuccess(
                    tapSignerPreviewNew(preview: true),
                    setup.deriveInfo
                )
            )
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-tap-signer-import-success")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "tap-signer-import-success")
        try assertLightScreenBottomEdgeIsClear(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("continue"),
            "expected compact TapSigner import success screen to show Continue, got:\n\(recognizedText)"
        )
    }

    func testCompactTapSignerRetryActionsAreVisible() async throws {
        try await bootstrapIfNeeded()

        try assertTapSignerRoute(
            .importRetry(tapSignerPreviewNew(preview: true)),
            screenName: "tap-signer-import-retry",
            expectedText: "retry"
        )
        try assertTapSignerRoute(
            .setupRetry(
                tapSignerPreviewNew(preview: true),
                tapSignerSetupRetryContinueCmd(preview: true)
            ),
            screenName: "tap-signer-setup-retry",
            expectedText: "retry"
        )
    }

    func testCompactTapSignerChainCodeActionsAreVisible() async throws {
        try await bootstrapIfNeeded()

        try assertTapSignerRoute(
            .initSelect(tapSignerPreviewNew(preview: true)),
            screenName: "tap-signer-choose-chain-code",
            expectedText: "advanced setup"
        )
        try assertTapSignerRoute(
            .initAdvanced(tapSignerPreviewNew(preview: true)),
            screenName: "tap-signer-advanced-chain-code",
            expectedText: "generate new string for me",
            requiresBottomButtonBand: true
        )
    }

    func testCompactOnboardingSecretWordsActionIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: OnboardingSecretWordsView(
                words: (1 ... 12).map { "word-\($0)" },
                onBack: {},
                onSaved: {}
            )
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-onboarding-secret-words")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "onboarding-backup-views")
        try assertPrimaryActionIsNotClippedAtBottom(in: image)
        try assertBluePrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("i saved these words"),
            "expected compact onboarding secret words screen to show I Saved These Words, got:\n\(recognizedText)"
        )
    }

    func testCompactCloudBackupEnableCanScrollToAction() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let view = CloudBackupEnableOnboardingView(
            onEnable: {},
            onCancel: {},
            message: nil,
            isBusy: false
        )
        .frame(width: size.width, height: size.height)
        let image = try renderAfterScrollingToBottom(view: view, size: size)
        addScreenshotAttachment(image, name: "compact-cloud-backup-enable-onboarding")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "cloud-backup-enable-onboarding")
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("enable cloud backup"),
            "expected compact cloud backup onboarding screen to show Enable Cloud Backup after scrolling, got:\n\(recognizedText)"
        )
    }

    func testCompactFeeRateSheetActionIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let feeOptions = FeeRateOptionsWithTotalFee.previewNew()
        let image = render(
            view: SendFlowSelectFeeRateView(
                manager: WalletManager(preview: "preview_only"),
                feeOptions: .constant(feeOptions),
                selectedOption: .constant(feeOptions.medium()),
                selectedPresentationDetent: .constant(.large)
            )
            .environment(AppManager.shared)
            .frame(height: 440)
            .frame(width: size.width, height: size.height, alignment: .top)
            .background(Color.coveBg),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-fee-rate-sheet")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "send-flow-select-fee-rate")
        try assertLightScreenBottomEdgeIsClear(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("customize fee"),
            "expected compact fee-rate sheet to show Customize Fee, got:\n\(recognizedText)"
        )
    }

    func testCompactSendFlowConfirmActionIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let manager = WalletManager(preview: "preview_only")
        let presenter = SendFlowPresenter(app: AppManager.shared, manager: manager)
        let image = render(
            view: NavigationStack {
                SendFlowConfirmScreen(
                    id: WalletId(),
                    manager: manager,
                    details: confirmDetailsPreviewNew(),
                    input: .unsigned,
                    payjoinEndpoint: nil
                )
                .environment(AppManager.shared)
                .environment(AuthManager.shared)
                .environment(presenter)
            }
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-send-flow-confirm")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "send-flow-confirm")
        try assertLightScreenBottomEdgeIsClear(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("swipe to send"),
            "expected compact send confirmation screen to show Swipe to Send, got:\n\(recognizedText)"
        )
    }

    func testLargeDynamicTypeSendFlowConfirmActionIsVisibleOnTallViewport() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 430, height: 932)
        let manager = WalletManager(preview: "preview_only")
        let presenter = SendFlowPresenter(app: AppManager.shared, manager: manager)
        let image = render(
            view: NavigationStack {
                SendFlowConfirmScreen(
                    id: WalletId(),
                    manager: manager,
                    details: confirmDetailsPreviewNew(),
                    input: .unsigned,
                    payjoinEndpoint: nil
                )
                .environment(AppManager.shared)
                .environment(AuthManager.shared)
                .environment(presenter)
            }
            .environment(\.sizeCategory, .accessibilityExtraExtraLarge)
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "large-dynamic-type-send-flow-confirm")
        try saveAuditScreenshotIfDirectoryRequested(
            image,
            name: "send-flow-confirm-large-dynamic-type-after.png"
        )
        try assertLightScreenBottomEdgeIsClear(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("swipe to send"),
            "expected large Dynamic Type send confirmation screen to show Swipe to Send, got:\n\(recognizedText)"
        )
    }

    func testCompactHotWalletImportActionIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: NavigationStack {
                HotWalletImportScreen(numberOfWords: .twelve)
                    .environment(AppManager.shared)
            }
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-hot-wallet-import")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "hot-wallet-import")
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("import wallet"),
            "expected compact hot wallet import screen to show Import Wallet, got:\n\(recognizedText)"
        )
    }

    func testCompactVerifyWordsActionsAreVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let view = NavigationStack {
            VerifyWordsScreen(
                manager: WalletManager(preview: "preview_only"),
                stateMachine: WordVerifyStateMachine(
                    validator: WordValidator.preview(preview: true),
                    startingWordNumber: 1
                ),
                verificationComplete: .constant(false)
            )
            .environment(AppManager.shared)
        }
        .frame(width: size.width, height: size.height)
        let image = render(view: view, size: size)
        addScreenshotAttachment(image, name: "compact-verify-words")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "verify-words")
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("show words"),
            "expected compact verify words screen to show Show Words, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            recognizedText.contains("skip verification"),
            "expected compact verify words screen to show Skip Verification, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            recognizedText.contains("what is word"),
            "expected compact verify words screen to keep the answer prompt visible, got:\n\(recognizedText)"
        )
    }

    func testCompactSecretWordsContentIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: NavigationStack {
                SecretWordsScreen(
                    id: WalletId(),
                    words: Mnemonic.preview(numberOfBip39Words: .twentyFour)
                )
                .environment(AppManager.shared)
                .environment(AuthManager.shared)
            }
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-secret-words")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "secret-words")
        try assertRecoveryWordCardsStayInsideHorizontalViewport(in: image)
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("recovery words"),
            "expected compact secret words screen to show Recovery Words, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            recognizedText.contains("please save these words"),
            "expected compact secret words screen to show the save guidance, got:\n\(recognizedText)"
        )
    }

    func testCompactUtxoListActionIsVisible() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: NavigationStack {
                UtxoListScreen(
                    manager: CoinControlManager(RustCoinControlManager.previewNew())
                )
                .environment(WalletManager(preview: "preview_only"))
            }
            .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-utxo-list")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: "utxo-list")
        try assertLightScreenBottomButtonBandIsVisible(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("manage utxos"),
            "expected compact UTXO list screen to render Manage UTXOs, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            recognizedText.contains("continue"),
            "expected compact UTXO list screen to show Continue, got:\n\(recognizedText)"
        )
    }

    func testCompactRecoveryWordsLayoutCanScrollToPrimaryAction() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 375, height: 667)
        let manager = PendingWalletManager(numberOfWords: .twentyFour)
        let initialTabIndex =
            screenshotMode() == "initial"
                ? 0
                : manager.rust.bip39WordsGrouped().count - 1
        let view = NavigationStack {
            WordsView(manager: manager, initialTabIndex: initialTabIndex)
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
            try assertNavigationChromeDoesNotShowWordCardBackground(in: image)
            try assertRecoveryWordCardsStayInsideHorizontalViewport(in: image)
            try assertPrimaryActionIsNotClippedAtBottom(in: image)

            let recognizedText = try normalizedRecognizedText(in: image)

            XCTAssertTrue(
                recognizedText.contains("next"),
                "expected compact recovery words initial screen to show Next, got:\n\(recognizedText)"
            )
            return
        }

        let scrollView = try XCTUnwrap(findVerticallyScrollableView(in: hostingController.view))
        let maxOffsetY = max(
            scrollView.contentSize.height - scrollView.bounds.height + scrollView.adjustedContentInset.bottom,
            -scrollView.adjustedContentInset.top
        )
        scrollView.setContentOffset(CGPoint(x: 0, y: maxOffsetY), animated: false)
        hostingController.view.layoutIfNeeded()

        let image = render(hostingController: hostingController, size: size)
        addScreenshotAttachment(image, name: "compact-recovery-words-after-scroll")
        try saveScreenshotIfRequested(image)
        try saveAuditScreenshotIfDirectoryRequested(image, name: "hot-wallet-create-after.png")
        try assertNavigationChromeDoesNotShowWordCardBackground(in: image)
        try assertRecoveryWordCardsStayInsideHorizontalViewport(in: image)
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("save wallet"),
            "expected compact recovery words screen to show Save Wallet, got:\n\(recognizedText)"
        )
        XCTAssertTrue(
            recognizedText.contains("recovery words"),
            "expected compact recovery words screen to keep the word-copy context visible, got:\n\(recognizedText)"
        )
    }

    func testLargeRecoveryWordsLayoutKeepsWordCardsInsideViewport() async throws {
        try await bootstrapIfNeeded()

        let size = CGSize(width: 430, height: 932)
        let manager = PendingWalletManager(numberOfWords: .twentyFour)
        let view = NavigationStack {
            WordsView(
                manager: manager,
                initialTabIndex: manager.rust.bip39WordsGrouped().count - 1
            )
            .environment(AppManager.shared)
        }
        .frame(width: size.width, height: size.height)
        let image = try renderAfterScrollingToBottom(view: view, size: size)
        addScreenshotAttachment(image, name: "large-recovery-words-after-scroll")
        try saveAuditScreenshotIfDirectoryRequested(image, name: "hot-wallet-create-large-after.png")
        try assertNavigationChromeDoesNotShowWordCardBackground(in: image)
        try assertRecoveryWordCardsStayInsideHorizontalViewport(in: image)
        try assertPrimaryActionIsNotClippedAtBottom(in: image)

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains("save wallet"),
            "expected large recovery words screen to show Save Wallet, got:\n\(recognizedText)"
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

    private func assertTapSignerRoute(
        _ route: TapSignerRoute,
        screenName: String,
        expectedText: String,
        requiresBottomButtonBand: Bool = false
    ) throws {
        let size = CGSize(width: 375, height: 667)
        let image = render(
            view: TapSignerContainer(route: route)
                .frame(width: size.width, height: size.height),
            size: size
        )
        addScreenshotAttachment(image, name: "compact-\(screenName)")
        try saveAuditScreenshotIfDirectoryRequested(image, screenName: screenName)
        try assertLightScreenBottomEdgeIsClear(in: image)
        if requiresBottomButtonBand {
            try assertLightScreenBottomButtonBandIsVisible(in: image)
        }

        let recognizedText = try normalizedRecognizedText(in: image)

        XCTAssertTrue(
            recognizedText.contains(expectedText),
            "expected compact \(screenName) screen to show \(expectedText), got:\n\(recognizedText)"
        )
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

    private func render(view: some View, size: CGSize) -> UIImage {
        let hostingController = hostingController(rootView: view, size: size)

        return render(hostingController: hostingController, size: size)
    }

    private func renderAfterScrollingToBottom(view: some View, size: CGSize) throws -> UIImage {
        let hostingController = hostingController(rootView: view, size: size)
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

        return render(hostingController: hostingController, size: size)
    }

    private func hostingController(rootView: some View, size: CGSize) -> UIHostingController<some View> {
        let hostingController = UIHostingController(rootView: rootView)
        let window = UIWindow(frame: CGRect(origin: .zero, size: size))
        window.rootViewController = hostingController
        window.makeKeyAndVisible()

        hostingController.view.bounds = window.bounds
        hostingController.view.backgroundColor = .clear
        hostingController.view.setNeedsLayout()
        hostingController.view.layoutIfNeeded()

        return hostingController
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
        let scale = CGFloat(cgImage.width) / image.size.width
        let sampleHeight = min(Int(16 * scale), cgImage.height)
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

    private func assertBluePrimaryActionIsNotClippedAtBottom(
        in image: UIImage,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let cgImage = try XCTUnwrap(image.cgImage)
        let scale = CGFloat(cgImage.width) / image.size.width
        let sampleHeight = min(Int(16 * scale), cgImage.height)
        let bottomRect = CGRect(
            x: 0,
            y: cgImage.height - sampleHeight,
            width: cgImage.width,
            height: sampleHeight
        )
        let bottomEdge = try XCTUnwrap(cgImage.cropping(to: bottomRect))
        let pixels = try rgbaPixels(in: bottomEdge)
        let blueButtonPixelCount = stride(from: 0, to: pixels.count, by: 4).count { offset in
            isPrimaryBluePixel(
                red: pixels[offset],
                green: pixels[offset + 1],
                blue: pixels[offset + 2]
            )
        }
        let maximumAllowedBluePixels = max(1, bottomEdge.width * bottomEdge.height / 80)

        XCTAssertLessThan(
            blueButtonPixelCount,
            maximumAllowedBluePixels,
            "expected dark padding below the blue primary action; blue pixels at the bottom edge indicate the button is clipped",
            file: file,
            line: line
        )
    }

    private func isPrimaryBluePixel(red: UInt8, green: UInt8, blue: UInt8) -> Bool {
        Int(blue) > 145 &&
            Int(blue) > Int(red) + 45 &&
            Int(blue) > Int(green) + 20 &&
            Int(green) > 45
    }

    private func assertLightScreenBottomEdgeIsClear(
        in image: UIImage,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let cgImage = try XCTUnwrap(image.cgImage)
        let scale = CGFloat(cgImage.width) / image.size.width
        let sampleHeight = min(Int(16 * scale), cgImage.height)
        let bottomRect = CGRect(
            x: 0,
            y: cgImage.height - sampleHeight,
            width: cgImage.width,
            height: sampleHeight
        )
        let bottomEdge = try XCTUnwrap(cgImage.cropping(to: bottomRect))
        let pixels = try rgbaPixels(in: bottomEdge)
        let nonLightPixelCount = stride(from: 0, to: pixels.count, by: 4).count(where: { offset in
            pixels[offset] < 235 ||
                pixels[offset + 1] < 235 ||
                pixels[offset + 2] < 235
        })
        let maximumAllowedNonLightPixels = max(1, bottomEdge.width * bottomEdge.height / 20)

        XCTAssertLessThan(
            nonLightPixelCount,
            maximumAllowedNonLightPixels,
            "expected clear light padding below the primary action; dark or colored pixels at the bottom edge indicate clipped content",
            file: file,
            line: line
        )
    }

    private func assertLightScreenBottomButtonBandIsVisible(
        in image: UIImage,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let cgImage = try XCTUnwrap(image.cgImage)
        let scale = CGFloat(cgImage.width) / image.size.width
        let bandRect = CGRect(
            x: Int(16 * scale),
            y: Int((image.size.height - 112) * scale),
            width: Int((image.size.width - 32) * scale),
            height: Int(80 * scale)
        )
        let actionBand = try XCTUnwrap(cgImage.cropping(to: bandRect))
        let pixels = try rgbaPixels(in: actionBand)
        let neutralButtonPixelCount = stride(from: 0, to: pixels.count, by: 4).count { offset in
            let red = Int(pixels[offset])
            let green = Int(pixels[offset + 1])
            let blue = Int(pixels[offset + 2])
            let channelSpread = max(red, green, blue) - min(red, green, blue)

            return red > 165 &&
                red < 230 &&
                green > 165 &&
                green < 230 &&
                blue > 165 &&
                blue < 235 &&
                channelSpread < 18
        }
        let minimumButtonPixels = actionBand.width * actionBand.height / 10

        XCTAssertGreaterThan(
            neutralButtonPixelCount,
            minimumButtonPixels,
            "expected the disabled bottom action band to be visible in the compact viewport",
            file: file,
            line: line
        )
    }

    private func assertNavigationChromeDoesNotShowWordCardBackground(
        in image: UIImage,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let cgImage = try XCTUnwrap(image.cgImage)
        let scale = CGFloat(cgImage.width) / image.size.width
        let navBand = CGRect(
            x: 0,
            y: Int(64 * scale),
            width: cgImage.width,
            height: Int(56 * scale)
        )
        let croppedBand = try XCTUnwrap(cgImage.cropping(to: navBand))
        let pixels = try rgbaPixels(in: croppedBand)
        let lightWordCardPixelCount = lightWordCardPixelCount(in: pixels)
        let maximumAllowedLightPixels = max(1, croppedBand.width * croppedBand.height / 25)

        XCTAssertLessThan(
            lightWordCardPixelCount,
            maximumAllowedLightPixels,
            "expected solid navigation chrome; light recovery word card pixels in the nav band indicate content is scrolling under the title/back button",
            file: file,
            line: line
        )
    }

    private func assertRecoveryWordCardsStayInsideHorizontalViewport(
        in image: UIImage,
        file: StaticString = #filePath,
        line: UInt = #line
    ) throws {
        let cgImage = try XCTUnwrap(image.cgImage)
        let scale = CGFloat(cgImage.width) / image.size.width
        let edgeWidth = max(Int(8 * scale), 1)
        let bandTop = max(Int(110 * scale), 0)
        let bandHeight = min(Int(300 * scale), cgImage.height - bandTop)
        let leftEdge = CGRect(x: 0, y: bandTop, width: edgeWidth, height: bandHeight)
        let rightEdge = CGRect(
            x: cgImage.width - edgeWidth,
            y: bandTop,
            width: edgeWidth,
            height: bandHeight
        )
        let leftCardPixels = try lightWordCardPixelCount(in: cgImage, rect: leftEdge)
        let rightCardPixels = try lightWordCardPixelCount(in: cgImage, rect: rightEdge)
        let maximumAllowedEdgePixels = max(1, edgeWidth * bandHeight / 25)

        XCTAssertLessThan(
            leftCardPixels,
            maximumAllowedEdgePixels,
            "expected recovery word cards to stay inside the left viewport edge",
            file: file,
            line: line
        )
        XCTAssertLessThan(
            rightCardPixels,
            maximumAllowedEdgePixels,
            "expected recovery word cards to stay inside the right viewport edge",
            file: file,
            line: line
        )
    }

    private func lightWordCardPixelCount(in image: CGImage, rect: CGRect) throws -> Int {
        let croppedImage = try XCTUnwrap(image.cropping(to: rect))
        let pixels = try rgbaPixels(in: croppedImage)

        return lightWordCardPixelCount(in: pixels)
    }

    private func lightWordCardPixelCount(in pixels: [UInt8]) -> Int {
        var count = 0
        var offset = 0

        while offset + 2 < pixels.count {
            if isLightWordCardPixel(
                red: pixels[offset],
                green: pixels[offset + 1],
                blue: pixels[offset + 2]
            ) {
                count += 1
            }

            offset += 4
        }

        return count
    }

    private func isLightWordCardPixel(red: UInt8, green: UInt8, blue: UInt8) -> Bool {
        let red = Int(red)
        let green = Int(green)
        let blue = Int(blue)

        return red > 190 &&
            red < 245 &&
            green > 195 &&
            green < 245 &&
            blue > 200 &&
            blue < 245
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

        try saveAuditScreenshot(image, name: name)
    }

    private func saveAuditScreenshotIfDirectoryRequested(_ image: UIImage, name: String) throws {
        guard screenshotEnvironmentValue("COVE_LAYOUT_SCREENSHOT_DIR") != nil else {
            return
        }

        try saveAuditScreenshot(image, name: name)
    }

    private func saveAuditScreenshotIfDirectoryRequested(_ image: UIImage, screenName: String) throws {
        guard screenshotEnvironmentValue("COVE_LAYOUT_SCREENSHOT_DIR") != nil else {
            return
        }

        let phase = screenshotEnvironmentValue("COVE_LAYOUT_SCREENSHOT_PHASE") ?? "after"
        try saveAuditScreenshot(image, name: "\(screenName)-\(phase).png")
    }

    private func saveAuditScreenshot(_ image: UIImage, name: String) throws {
        let screenshotDirectory: URL
        if let directoryPath = screenshotEnvironmentValue("COVE_LAYOUT_SCREENSHOT_DIR") {
            screenshotDirectory = URL(fileURLWithPath: directoryPath)
        } else {
            let documentsDirectory = try FileManager.default.url(
                for: .documentDirectory,
                in: .userDomainMask,
                appropriateFor: nil,
                create: true
            )
            screenshotDirectory = documentsDirectory.appendingPathComponent("layout-screenshots")
        }
        try FileManager.default.createDirectory(at: screenshotDirectory, withIntermediateDirectories: true)

        let screenshotUrl = screenshotDirectory.appendingPathComponent(name)
        try XCTUnwrap(image.pngData()).write(to: screenshotUrl)
    }

    private func screenshotEnvironmentValue(_ key: String) -> String? {
        let environment = ProcessInfo.processInfo.environment

        return environment[key] ?? environment["TEST_RUNNER_\(key)"]
    }
}
