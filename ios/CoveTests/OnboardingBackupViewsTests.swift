@testable import Cove
import CoveCore
import XCTest

final class OnboardingBackupViewsTests: XCTestCase {
    func testScP04EveryEnableStateProjectsItsExpectedBusyCategory() {
        let context = CloudBackupEnableContext(
            savedPasskeyConfirmation: .manual,
            verificationSource: .settings
        )
        let hidden = CloudBackupVerificationPresentation.hidden(source: nil)
        let progress = CloudBackupProgress(completed: 1, total: 2)
        let defaultCopy = enableBusyCopy(nil, verificationPresentation: hidden)
        let checkingCopy = enableBusyCopy(
            .waitingForPasskeyAvailability,
            verificationPresentation: hidden
        )
        let uploadCopy = enableBusyCopy(
            .uploadingInitialBackup(progress: progress),
            verificationPresentation: hidden
        )
        XCTAssertEqual(defaultCopy.title, "Checking for existing backups...")
        XCTAssertEqual(uploadCopy.title, "Creating your encrypted backup...")

        let promptOrDiscoveryStates: [CloudBackupEnableFlow?] = [
            nil,
            .discoveringExistingBackup,
            .awaitingForceNewConfirmation(context, nil),
            .awaitingPasskeyChoice(.enable(context, nil)),
        ]
        for state in promptOrDiscoveryStates {
            XCTAssertEqual(enableBusyCopy(state, verificationPresentation: hidden), defaultCopy)
        }

        let savedPasskeyStates: [CloudBackupEnableFlow] = [
            .waitingForPasskeyAvailability,
            .awaitingSavedPasskeyConfirmation(.automatic),
            .awaitingSavedPasskeyConfirmation(.manual),
        ]
        for state in savedPasskeyStates {
            XCTAssertEqual(enableBusyCopy(state, verificationPresentation: hidden), checkingCopy)
        }

        let uploadStates: [CloudBackupEnableFlow] = [
            .uploadingInitialBackup(progress: progress),
            .retryingUploadWithStagedMaterial(progress: progress),
        ]
        for state in uploadStates {
            XCTAssertEqual(enableBusyCopy(state, verificationPresentation: hidden), uploadCopy)
        }

        let creatingCopy = enableBusyCopy(
            .creatingPasskey,
            verificationPresentation: hidden
        )
        let confirmingCopy = enableBusyCopy(
            .confirmingSavedPasskey,
            verificationPresentation: hidden
        )
        XCTAssertNotEqual(creatingCopy, defaultCopy)
        XCTAssertNotEqual(confirmingCopy, creatingCopy)
        XCTAssertNotEqual(confirmingCopy, checkingCopy)
        XCTAssertEqual(uploadCopy.progress, progress)
        XCTAssertNil(defaultCopy.progress)
        XCTAssertNil(checkingCopy.progress)
        XCTAssertNil(creatingCopy.progress)
        XCTAssertNil(confirmingCopy.progress)
    }

    func testScP04EveryBatchStateProjectsItsControl() {
        XCTAssertEqual(cloudBackupRestoreAllPresentation(state: .notShown), .hidden)

        // whether the control can be tapped is the screen's actions, so a disabled and an
        // available state draw the same control
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .startDisabled(walletCount: 2)),
            .start(title: "Restore All (2)")
        )
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .startAvailable(walletCount: 2)),
            .start(title: "Restore All (2)")
        )
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .retryDisabled(walletCount: 1)),
            .retry(title: "Retry Remaining (1)")
        )
        XCTAssertEqual(
            cloudBackupRestoreAllPresentation(state: .retryAvailable(walletCount: 1)),
            .retry(title: "Retry Remaining (1)")
        )

        guard case let .running(running) = cloudBackupRestoreAllPresentation(
            state: .running(
                completed: 0,
                total: 2,
                currentWalletName: nil,
                cancellationRequested: false
            )
        ) else {
            return XCTFail("expected running presentation")
        }
        XCTAssertFalse(running.cancellationRequested)
        XCTAssertEqual(running.accessibilityValue, "Preparing wallet restores, Completed 0 of 2")

        guard case let .running(cancelling) = cloudBackupRestoreAllPresentation(
            state: .running(
                completed: 1,
                total: 2,
                currentWalletName: "Savings",
                cancellationRequested: true
            )
        ) else {
            return XCTFail("expected cancelling presentation")
        }
        XCTAssertTrue(cancelling.cancellationRequested)
        XCTAssertEqual(
            cancelling.accessibilityValue,
            "Restoring Savings, Completed 1 of 2, Cancel requested. The current wallet will finish first."
        )
    }

    func testRecoveryWordsUseFirstHalfInLeftColumn() {
        let words = (1 ... 12).map { "word-\($0)" }

        let orderedWords = onboardingWordsInTwoColumnVisualOrder(words)

        XCTAssertEqual(
            [
                OnboardingWordCardItem(index: 1, word: "word-1"),
                OnboardingWordCardItem(index: 7, word: "word-7"),
                OnboardingWordCardItem(index: 2, word: "word-2"),
                OnboardingWordCardItem(index: 8, word: "word-8"),
                OnboardingWordCardItem(index: 3, word: "word-3"),
                OnboardingWordCardItem(index: 9, word: "word-9"),
                OnboardingWordCardItem(index: 4, word: "word-4"),
                OnboardingWordCardItem(index: 10, word: "word-10"),
                OnboardingWordCardItem(index: 5, word: "word-5"),
                OnboardingWordCardItem(index: 11, word: "word-11"),
                OnboardingWordCardItem(index: 6, word: "word-6"),
                OnboardingWordCardItem(index: 12, word: "word-12"),
            ],
            orderedWords
        )
    }

    func testOnboardingEnableCompletionOnlyAcceptsOnboardingContext() {
        XCTAssertTrue(
            isOnboardingCloudBackupEnableCompletion(.init(
                savedPasskeyConfirmation: .automatic,
                verificationSource: .onboarding
            ))
        )
        XCTAssertFalse(
            isOnboardingCloudBackupEnableCompletion(.init(
                savedPasskeyConfirmation: .manual,
                verificationSource: .settings
            ))
        )
    }

    func testOnboardingRelaunchFallbackRequiresRustOwnedDurableReadiness() {
        XCTAssertTrue(shouldCompleteOnboardingCloudBackupFromPersistedState(.ready))
        XCTAssertFalse(shouldCompleteOnboardingCloudBackupFromPersistedState(.notReady))
        XCTAssertFalse(
            shouldCompleteOnboardingCloudBackupFromPersistedState(.pendingEnableRecovery)
        )
    }

    func testPendingEnableRecoveryReplacesOnboardingEnablePresentation() {
        let recovery = CloudBackupPendingEnableRecovery(
            supportCode: "CB-PE-001",
            cleanup: .available
        )

        XCTAssertEqual(
            onboardingCloudBackupStepPresentation(
                body: .pendingEnableRecovery(recovery)
            ),
            .pendingEnableRecovery(recovery)
        )
        XCTAssertEqual(
            onboardingCloudBackupStepPresentation(body: .disabled),
            .enable
        )
    }

    /// An Enable paused on its saved passkey keeps onboarding on Confirm Passkey, and only an
    /// Enable's pause does: other paused work isn't onboarding's to continue
    func testPausedEnableMakesTheOnboardingPrimaryButtonConfirmThePasskey() throws {
        let fixtures = cloudBackupScreenFixtures()
        let screen = { (name: String) in
            try XCTUnwrap(fixtures.first { $0.name.hasSuffix(":\(name)") }?.screen)
        }

        let pausedEnable = try screen("saved-passkey-confirmation-enable")
        XCTAssertEqual(onboardingCloudBackupPrimaryAction(screen: pausedEnable), .confirmSavedPasskey)
        XCTAssertTrue(pausedEnable.actions.isAvailable(action: .confirmSavedPasskey))

        let pausedReinitialize = try screen("saved-passkey-confirmation-reinitialize")
        XCTAssertEqual(onboardingCloudBackupPrimaryAction(screen: pausedReinitialize), .enable)

        let disabled = try screen("disabled")
        XCTAssertEqual(onboardingCloudBackupPrimaryAction(screen: disabled), .enable)
    }

    func testPendingEnableRecoveryRoutesCleanupToManagerAndCancelToSkip() {
        var dispatched: CloudBackupManagerAction?
        var didSkip = false

        routeOnboardingCloudBackupRecoveryIntent(
            .removeIncompleteSetup,
            dispatch: { dispatched = $0 },
            onSkip: { didSkip = true }
        )

        guard case .confirmPendingEnableCleanup = dispatched else {
            return XCTFail("expected pending-enable cleanup action")
        }
        XCTAssertFalse(didSkip)

        dispatched = nil
        routeOnboardingCloudBackupRecoveryIntent(
            .skip,
            dispatch: { dispatched = $0 },
            onSkip: { didSkip = true }
        )

        XCTAssertNil(dispatched)
        XCTAssertTrue(didSkip)
    }

    private func enableBusyCopy(
        _ flow: CloudBackupEnableFlow?,
        verificationPresentation: CloudBackupVerificationPresentation
    ) -> CloudBackupEnableBusyCopy {
        cloudBackupEnableBusyCopy(
            enableFlow: flow,
            verificationPresentation: verificationPresentation
        )
    }
}
