@testable import Cove
import XCTest

final class PresentationTransitionCoordinatorTests: XCTestCase {
    private enum Presentation {
        case first
        case second
        case sensitive
    }

    @MainActor
    func testPresentWhileActiveWaitsForPresenterReadiness() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()

        coordinator.present(.first)
        coordinator.present(.second)

        XCTAssertNil(coordinator.currentPresentation)
        guard case .second = coordinator.queuedPresentation else {
            return XCTFail("Expected the second presentation to be queued")
        }

        let readinessRequestID = try XCTUnwrap(coordinator.readinessRequestID)
        XCTAssertEqual(
            coordinator.hostState,
            .awaitingPresenterReadiness(readinessRequestID)
        )

        coordinator.presenterDidBecomeReady(readinessRequestID)

        guard case .second = coordinator.currentPresentation?.item else {
            return XCTFail("Expected the second presentation after readiness")
        }
        XCTAssertNil(coordinator.queuedPresentation)
        XCTAssertFalse(coordinator.isAwaitingPresenterReadiness)
    }

    @MainActor
    func testStaleFalseBindingWriteDoesNotDismissNewActiveSlot() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()

        coordinator.present(.first)
        let staleBinding = coordinator.isPresented { _ in true }
        coordinator.present(.second)
        try coordinator.presenterDidBecomeReady(XCTUnwrap(coordinator.readinessRequestID))

        staleBinding.wrappedValue = false

        guard case .second = coordinator.currentPresentation?.item else {
            return XCTFail("Expected the new active slot to ignore the stale false write")
        }
    }

    @MainActor
    func testStaleNilItemWriteDoesNotDismissNewActiveSlot() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()

        coordinator.present(.first)
        let staleBinding = coordinator.presentedItem { _ in true }
        coordinator.present(.second)
        try coordinator.presenterDidBecomeReady(XCTUnwrap(coordinator.readinessRequestID))

        staleBinding.wrappedValue = nil

        guard case .second = coordinator.currentPresentation?.item else {
            return XCTFail("Expected the new active slot to ignore the stale nil write")
        }
    }

    @MainActor
    func testDiscardRemovesMatchingCurrentPresentation() {
        let coordinator = PresentationTransitionCoordinator<Presentation>()

        coordinator.present(.sensitive)
        coordinator.discard { presentation in
            if case .sensitive = presentation { true } else { false }
        }

        XCTAssertNil(coordinator.currentPresentation)
        XCTAssertNil(coordinator.queuedPresentation)
        XCTAssertTrue(coordinator.isAwaitingPresenterReadiness)
    }

    @MainActor
    func testDiscardRemovesCurrentXprvPresentation() {
        let coordinator = PresentationTransitionCoordinator<WalletSettingsPresentationState>()

        coordinator.present(.xprvReveal)
        coordinator.discard { $0.slot == .xprvReveal }

        XCTAssertNil(coordinator.currentPresentation)
        XCTAssertNil(coordinator.queuedPresentation)
        XCTAssertTrue(coordinator.isAwaitingPresenterReadiness)
    }

    @MainActor
    func testHostDisappearanceClearsPendingTransition() {
        let coordinator = PresentationTransitionCoordinator<Presentation>()

        coordinator.present(.first)
        coordinator.present(.second)
        coordinator.hostDidDisappear()

        XCTAssertNil(coordinator.currentPresentation)
        XCTAssertNil(coordinator.queuedPresentation)
        XCTAssertFalse(coordinator.isAwaitingPresenterReadiness)
        XCTAssertEqual(coordinator.hostState, .idle)
    }

    @MainActor
    func testStaleReadinessSignalCannotAdvanceANewPresentation() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()

        coordinator.present(.first)
        coordinator.present(.second)
        let staleRequestID = try XCTUnwrap(coordinator.readinessRequestID)
        coordinator.hostDidDisappear()
        coordinator.present(.sensitive)

        coordinator.presenterDidBecomeReady(staleRequestID)

        guard case .sensitive = coordinator.currentPresentation?.item else {
            return XCTFail("Expected a stale readiness signal to leave the new presentation active")
        }
    }

    @MainActor
    func testPendingActionDoesNotDispatchBeforeReadiness() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()
        let handoff = PresentationActionHandoff<Presentation, Presentation>()
        coordinator.present(.first)
        let transition = try XCTUnwrap(
            coordinator.dismissCurrentPresentationForTransition()
        )
        _ = handoff.stage(
            action: .sensitive,
            presentation: .first,
            transition: transition
        )

        let dispatched: [Presentation] = []

        XCTAssertNotNil(handoff.pendingAction)
        XCTAssertTrue(dispatched.isEmpty)
        XCTAssertTrue(coordinator.isAwaitingPresenterReadiness)
    }

    @MainActor
    func testPendingActionDispatchesExactlyOnceAfterMatchingReadiness() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()
        let handoff = PresentationActionHandoff<Presentation, Presentation>()
        coordinator.present(.first)
        let transition = try XCTUnwrap(
            coordinator.dismissCurrentPresentationForTransition()
        )
        _ = handoff.stage(
            action: .sensitive,
            presentation: .first,
            transition: transition
        )

        var dispatched: [Presentation] = []
        let handled = handoff.presenterDidBecomeReady(
            transition.readinessRequestID,
            currentPresentation: .first,
            isHostAvailable: true,
            using: coordinator
        ) { dispatched.append($0) }

        XCTAssertTrue(handled)
        XCTAssertEqual(dispatched, [.sensitive])
        XCTAssertNil(coordinator.currentPresentation)
        XCTAssertNil(coordinator.queuedPresentation)

        _ = handoff.presenterDidBecomeReady(
            transition.readinessRequestID,
            currentPresentation: .first,
            isHostAvailable: true,
            using: coordinator
        ) { dispatched.append($0) }

        XCTAssertEqual(dispatched, [.sensitive])
    }

    @MainActor
    func testStaleReadinessCancelsPendingAction() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()
        let handoff = PresentationActionHandoff<Presentation, Presentation>()
        coordinator.present(.first)
        let staleTransition = try XCTUnwrap(
            coordinator.dismissCurrentPresentationForTransition()
        )
        _ = handoff.stage(
            action: .sensitive,
            presentation: .first,
            transition: staleTransition
        )

        coordinator.hostDidDisappear()
        coordinator.present(.second)
        coordinator.present(.first)
        let currentTransition = try XCTUnwrap(coordinator.transitionRequest)
        var dispatched: [Presentation] = []

        XCTAssertTrue(
            handoff.presenterDidBecomeReady(
                staleTransition.readinessRequestID,
                currentPresentation: .first,
                isHostAvailable: true,
                using: coordinator
            ) { dispatched.append($0) }
        )
        XCTAssertNil(handoff.pendingAction)
        XCTAssertEqual(coordinator.transitionRequest, currentTransition)
        XCTAssertTrue(dispatched.isEmpty)
    }

    @MainActor
    func testUnrelatedReadinessDoesNotCancelPendingAction() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()
        let handoff = PresentationActionHandoff<Presentation, Presentation>()
        coordinator.present(.first)
        let transition = try XCTUnwrap(
            coordinator.dismissCurrentPresentationForTransition()
        )
        _ = handoff.stage(
            action: .sensitive,
            presentation: .first,
            transition: transition
        )

        var dispatched: [Presentation] = []
        XCTAssertFalse(
            handoff.presenterDidBecomeReady(
                UUID(),
                currentPresentation: .first,
                isHostAvailable: true,
                using: coordinator
            ) { dispatched.append($0) }
        )
        XCTAssertNotNil(handoff.pendingAction)
        XCTAssertTrue(dispatched.isEmpty)

        XCTAssertTrue(
            handoff.presenterDidBecomeReady(
                transition.readinessRequestID,
                currentPresentation: .first,
                isHostAvailable: true,
                using: coordinator
            ) { dispatched.append($0) }
        )
        XCTAssertEqual(dispatched, [.sensitive])
    }

    @MainActor
    func testChangedPromptCancelsPendingActionWithoutReopeningOldPrompt() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()
        let handoff = PresentationActionHandoff<Presentation, Presentation>()
        coordinator.present(.first)
        let transition = try XCTUnwrap(
            coordinator.dismissCurrentPresentationForTransition()
        )
        _ = handoff.stage(
            action: .sensitive,
            presentation: .first,
            transition: transition
        )

        var dispatched: [Presentation] = []
        XCTAssertTrue(
            handoff.presenterDidBecomeReady(
                transition.readinessRequestID,
                currentPresentation: .second,
                isHostAvailable: true,
                using: coordinator
            ) { dispatched.append($0) }
        )

        XCTAssertNil(handoff.pendingAction)
        XCTAssertNil(coordinator.currentPresentation)
        XCTAssertNil(coordinator.queuedPresentation)
        XCTAssertTrue(dispatched.isEmpty)
    }

    @MainActor
    func testHostDisappearanceCancelsPendingAction() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()
        let handoff = PresentationActionHandoff<Presentation, Presentation>()
        coordinator.present(.first)
        let transition = try XCTUnwrap(
            coordinator.dismissCurrentPresentationForTransition()
        )
        _ = handoff.stage(
            action: .sensitive,
            presentation: .first,
            transition: transition
        )

        handoff.cancel()
        coordinator.hostDidDisappear()

        XCTAssertNil(handoff.pendingAction)
        XCTAssertFalse(coordinator.hasPresentationActivity)
    }

    @MainActor
    func testMatchingReadinessDoesNotReopenQueuedOldPrompt() throws {
        let coordinator = PresentationTransitionCoordinator<Presentation>()
        let handoff = PresentationActionHandoff<Presentation, Presentation>()
        coordinator.present(.first)
        let transition = try XCTUnwrap(
            coordinator.dismissCurrentPresentationForTransition()
        )
        coordinator.queue(.first)
        _ = handoff.stage(
            action: .sensitive,
            presentation: .first,
            transition: transition
        )

        var dispatched: [Presentation] = []
        XCTAssertTrue(
            handoff.presenterDidBecomeReady(
                transition.readinessRequestID,
                currentPresentation: .first,
                isHostAvailable: true,
                using: coordinator
            ) { dispatched.append($0) }
        )

        XCTAssertEqual(dispatched, [.sensitive])
        XCTAssertNil(coordinator.currentPresentation)
        XCTAssertNil(coordinator.queuedPresentation)
    }
}
