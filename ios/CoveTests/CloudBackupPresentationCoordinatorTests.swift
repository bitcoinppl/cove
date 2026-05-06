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
                presentation: .existingBackupFound,
                context: context,
                hasBlockers: false
            )
        )
        XCTAssertTrue(
            isCloudBackupPresentationPresentable(
                presentation: .passkeyChoice(.enable),
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

    private func presentableContext(
        presentationPolicy: CloudBackupPresentationPolicy
    ) -> CloudBackupPresentationContext {
        CloudBackupPresentationContext(
            scenePhase: .active,
            isUnlocked: true,
            isCoverPresented: false,
            appHasAlert: false,
            appHasSheet: false,
            isViewingCloudBackup: false,
            presentationPolicy: presentationPolicy
        )
    }
}
