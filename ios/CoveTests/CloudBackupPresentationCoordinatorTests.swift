@testable import Cove
import CoveCore
import SwiftUI
import XCTest

final class CloudBackupPresentationCoordinatorTests: XCTestCase {
    func testOnboardingPolicySuppressesVerificationPrompt() {
        let context = presentableContext(presentationPolicy: .onboarding)

        XCTAssertFalse(
            isCloudBackupPresentationPresentable(
                presentation: .verificationPrompt,
                context: context,
                hasBlockers: false
            )
        )
    }

    func testOnboardingPolicySuppressesMissingPasskeyReminder() {
        let context = presentableContext(presentationPolicy: .onboarding)

        XCTAssertFalse(
            isCloudBackupPresentationPresentable(
                presentation: .missingPasskeyReminder,
                context: context,
                hasBlockers: false
            )
        )
    }

    func testOnboardingPolicyAllowsEnablePrompts() {
        let context = presentableContext(presentationPolicy: .onboarding)

        XCTAssertTrue(
            isCloudBackupPresentationPresentable(
                presentation: .existingBackupFound(cloudBackupEnableContext(), nil),
                context: context,
                hasBlockers: false
            )
        )
        XCTAssertTrue(
            isCloudBackupPresentationPresentable(
                presentation: .passkeyChoice(.enable(cloudBackupEnableContext(), nil)),
                context: context,
                hasBlockers: false
            )
        )
    }

    func testNormalPolicyAllowsVerificationPromptWhenUnblocked() {
        let context = presentableContext(presentationPolicy: .requiresUnlockedAuth)

        XCTAssertTrue(
            isCloudBackupPresentationPresentable(
                presentation: .verificationPrompt,
                context: context,
                hasBlockers: false
            )
        )
    }

    func testUnsettledNavigationBlocksNewVerificationPrompt() {
        let context = presentableContext(
            isNavigationSettled: false,
            presentationPolicy: .requiresUnlockedAuth
        )

        XCTAssertFalse(
            isCloudBackupPresentationPresentable(
                presentation: .verificationPrompt,
                context: context,
                hasBlockers: false
            )
        )
    }

    @MainActor
    func testUnsettledNavigationQueuesNewVerificationPrompt() {
        let coordinator = CloudBackupPresentationCoordinator {
            .verification
        }

        coordinator.update(
            context: presentableContext(
                isNavigationSettled: false,
                presentationPolicy: .requiresUnlockedAuth
            )
        )

        XCTAssertNil(coordinator.currentPresentation)
        XCTAssertEqual(coordinator.queuedPresentation, CloudBackupRootPresentation.verificationPrompt)
    }

    @MainActor
    func testQueuedVerificationPromptPresentsAfterNavigationSettles() {
        let coordinator = CloudBackupPresentationCoordinator {
            .verification
        }

        coordinator.update(
            context: presentableContext(
                isNavigationSettled: false,
                presentationPolicy: .requiresUnlockedAuth
            )
        )
        coordinator.update(
            context: presentableContext(
                isNavigationSettled: true,
                presentationPolicy: .requiresUnlockedAuth
            )
        )

        XCTAssertEqual(coordinator.currentPresentation, CloudBackupRootPresentation.verificationPrompt)
        XCTAssertNil(coordinator.queuedPresentation)
    }

    @MainActor
    func testVisibleVerificationPromptIsNotDismissedByNavigationSettling() {
        let coordinator = CloudBackupPresentationCoordinator {
            .verification
        }

        coordinator.update(context: presentableContext(presentationPolicy: .requiresUnlockedAuth))
        XCTAssertEqual(coordinator.currentPresentation, CloudBackupRootPresentation.verificationPrompt)

        coordinator.update(
            context: presentableContext(
                isNavigationSettled: false,
                presentationPolicy: .requiresUnlockedAuth
            )
        )

        XCTAssertEqual(coordinator.currentPresentation, CloudBackupRootPresentation.verificationPrompt)
    }

    @MainActor
    func testVisibleVerificationPromptDismissesForAppAlertDuringNavigationSettling() {
        let coordinator = CloudBackupPresentationCoordinator {
            .verification
        }

        coordinator.update(context: presentableContext(presentationPolicy: .requiresUnlockedAuth))
        XCTAssertEqual(coordinator.currentPresentation, CloudBackupRootPresentation.verificationPrompt)

        coordinator.update(
            context: presentableContext(
                appHasAlert: true,
                isNavigationSettled: false,
                presentationPolicy: .requiresUnlockedAuth
            )
        )

        XCTAssertNil(coordinator.currentPresentation)
        XCTAssertEqual(coordinator.queuedPresentation, CloudBackupRootPresentation.verificationPrompt)
    }

    func testRootPromptCompletionShowsSuccessFloaterFeedback() {
        XCTAssertEqual(
            cloudBackupVerificationFeedback(for: .completed(source: .rootPrompt)),
            .successFloater("Cloud Backup Verified")
        )
    }

    func testRootPromptFailureShowsAlertFeedback() {
        XCTAssertEqual(
            cloudBackupVerificationFeedback(
                for: .failed(source: .rootPrompt, message: "verification failed")
            ),
            .failureAlert(
                title: "Cloud Backup Verification Failed",
                message: "verification failed"
            )
        )
    }

    func testNonRootVerificationResultsDoNotShowGlobalFeedback() {
        let sources: [CloudBackupVerificationSource] = [
            .settings,
            .cloudBackupDetail,
            .onboarding,
        ]

        for source in sources {
            XCTAssertNil(
                cloudBackupVerificationFeedback(for: .completed(source: source)),
                "completed source \(source) should use its local UI"
            )
            XCTAssertNil(
                cloudBackupVerificationFeedback(
                    for: .failed(source: source, message: "verification failed")
                ),
                "failed source \(source) should use its local UI"
            )
        }
    }

    private func presentableContext(
        appHasAlert: Bool = false,
        isNavigationSettled: Bool = true,
        presentationPolicy: CloudBackupPresentationPolicy
    ) -> CloudBackupPresentationContext {
        CloudBackupPresentationContext(
            scenePhase: .active,
            isUnlocked: true,
            isCoverPresented: false,
            appHasAlert: appHasAlert,
            appHasSheet: false,
            isViewingCloudBackup: false,
            isNavigationSettled: isNavigationSettled,
            presentationPolicy: presentationPolicy
        )
    }

    private func cloudBackupEnableContext() -> CloudBackupEnableContext {
        CloudBackupEnableContext(
            savedPasskeyConfirmation: .manual,
            verificationSource: .settings
        )
    }
}
