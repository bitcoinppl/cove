@testable import Cove
import CoveCore
import XCTest

final class OnboardingBackupViewsTests: XCTestCase {
    func testScP04EveryInventoryStateProjectsRequiredActionAvailability() {
        let loaded = loadedCloudBackupDetail()
        let failure = "inventory unavailable"

        let notLoaded = CloudBackupDetailState.notLoaded
        XCTAssertNil(notLoaded.retainedDetailState)
        XCTAssertFalse(notLoaded.isChecking)
        XCTAssertFalse(notLoaded.isComplete)
        XCTAssertNil(notLoaded.inventoryError)

        let checking = CloudBackupDetailState.checking(retained: loaded)
        XCTAssertEqual(checking.retainedDetailState?.detail, loaded.detail)
        XCTAssertTrue(checking.isChecking)
        XCTAssertFalse(checking.isComplete)
        XCTAssertNil(checking.inventoryError)

        let failed = CloudBackupDetailState.failed(
            reason: .offline,
            error: failure,
            retained: loaded
        )
        XCTAssertEqual(failed.retainedDetailState?.detail, loaded.detail)
        XCTAssertFalse(failed.isChecking)
        XCTAssertFalse(failed.isComplete)
        XCTAssertEqual(failed.inventoryError, failure)

        let complete = CloudBackupDetailState.complete(state: loaded)
        XCTAssertEqual(complete.retainedDetailState?.detail, loaded.detail)
        XCTAssertFalse(complete.isChecking)
        XCTAssertTrue(complete.isComplete)
        XCTAssertNil(complete.inventoryError)
    }

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

    func testScP04EveryBatchStateProjectsRequiredActionAvailability() {
        XCTAssertEqual(cloudBackupRestoreAllPresentation(state: .notShown), .hidden)

        guard case .disabled = cloudBackupRestoreAllPresentation(
            state: .startDisabled(walletCount: 2)
        ) else {
            return XCTFail("expected unavailable start action")
        }
        guard case let .action(start) = cloudBackupRestoreAllPresentation(
            state: .startAvailable(walletCount: 2)
        ) else {
            return XCTFail("expected available start action")
        }
        XCTAssertEqual(start.intent, .start)

        guard case .disabled = cloudBackupRestoreAllPresentation(
            state: .retryDisabled(walletCount: 1)
        ) else {
            return XCTFail("expected unavailable retry action")
        }
        guard case let .action(retry) = cloudBackupRestoreAllPresentation(
            state: .retryAvailable(walletCount: 1)
        ) else {
            return XCTFail("expected available retry action")
        }
        XCTAssertEqual(retry.intent, .retry)

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
        XCTAssertTrue(running.canCancel)

        guard case let .running(cancelling) = cloudBackupRestoreAllPresentation(
            state: .running(
                completed: 0,
                total: 2,
                currentWalletName: nil,
                cancellationRequested: true
            )
        ) else {
            return XCTFail("expected cancelling presentation")
        }
        XCTAssertFalse(cancelling.canCancel)
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

    private func enableBusyCopy(
        _ flow: CloudBackupEnableFlow?,
        verificationPresentation: CloudBackupVerificationPresentation
    ) -> CloudBackupEnableBusyCopy {
        cloudBackupEnableBusyCopy(
            enableFlow: flow,
            verificationPresentation: verificationPresentation
        )
    }

    private func loadedCloudBackupDetail() -> LoadedCloudBackupDetail {
        LoadedCloudBackupDetail(
            detail: CloudBackupDetail(
                lastSync: nil,
                upToDate: [],
                needsSync: [],
                cloudOnlyCount: 1,
                otherBackups: .loaded(summary: CloudBackupOtherBackupsSummary(
                    namespaceCount: 0,
                    walletCount: 0,
                    passkeyHints: []
                ))
            ),
            cloudOnly: .notFetched,
            cloudOnlyOperation: .idle,
            otherBackupsOperation: .idle
        )
    }
}
