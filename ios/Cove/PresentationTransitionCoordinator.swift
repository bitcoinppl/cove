import SwiftUI
import UIKit

enum PresentationTransitionHostState: Equatable {
    case idle
    case presenting(UUID)
    case awaitingPresenterReadiness(UUID)
}

@MainActor
@Observable
final class PresentationTransitionCoordinator<Presentation> {
    private(set) var currentPresentation: TaggedItem<Presentation>?
    private(set) var queuedPresentation: Presentation?
    private(set) var readinessRequestID: UUID?

    var hostState: PresentationTransitionHostState {
        if let readinessRequestID {
            return .awaitingPresenterReadiness(readinessRequestID)
        }

        if let presentationID = currentPresentation?.id {
            return .presenting(presentationID)
        }

        return .idle
    }

    var isAwaitingPresenterReadiness: Bool {
        readinessRequestID != nil
    }

    var hasPresentationActivity: Bool {
        currentPresentation != nil || queuedPresentation != nil || readinessRequestID != nil
    }

    func present(_ presentation: Presentation) {
        guard currentPresentation == nil else {
            transition(to: presentation)
            return
        }

        guard readinessRequestID == nil else {
            queuedPresentation = presentation
            return
        }

        queuedPresentation = nil
        currentPresentation = TaggedItem(presentation)
    }

    func queue(_ presentation: Presentation) {
        queuedPresentation = presentation
    }

    func discardQueued(where shouldDiscard: (Presentation) -> Bool) {
        guard let queuedPresentation, shouldDiscard(queuedPresentation) else { return }

        self.queuedPresentation = nil
    }

    func transition(to presentation: Presentation) {
        queuedPresentation = presentation
        guard readinessRequestID == nil else { return }

        guard currentPresentation != nil else {
            presentQueuedPresentation()
            return
        }

        currentPresentation = nil
        beginWaitingForPresenterReadiness()
    }

    func transitionAfterPresenterDismissal(to presentation: Presentation) {
        queuedPresentation = presentation
        guard readinessRequestID == nil else { return }

        if currentPresentation != nil {
            currentPresentation = nil
        }

        beginWaitingForPresenterReadiness()
    }

    func dismissCurrentPresentation() {
        guard currentPresentation != nil else { return }

        currentPresentation = nil
        beginWaitingForPresenterReadiness()
    }

    func discard(where shouldDiscard: (Presentation) -> Bool) {
        if let queuedPresentation, shouldDiscard(queuedPresentation) {
            self.queuedPresentation = nil
        }

        guard
            let currentPresentation,
            shouldDiscard(currentPresentation.item)
        else { return }

        self.currentPresentation = nil
        beginWaitingForPresenterReadiness()
    }

    func discardAll() {
        currentPresentation = nil
        queuedPresentation = nil
        readinessRequestID = nil
    }

    func presenterDidBecomeReady(
        _ requestID: UUID,
        presentQueuedPresentation: Bool = true
    ) {
        guard readinessRequestID == requestID else { return }

        readinessRequestID = nil
        if presentQueuedPresentation {
            self.presentQueuedPresentation()
        }
    }

    func hostDidDisappear() {
        discardAll()
    }

    func isPresented(
        where matches: @escaping (Presentation) -> Bool
    ) -> Binding<Bool> {
        let observedPresentationID = currentPresentation.flatMap { presentation in
            matches(presentation.item) ? presentation.id : nil
        }

        return Binding(
            get: { [weak self] in
                guard let presentation = self?.currentPresentation?.item else {
                    return false
                }

                return matches(presentation)
            },
            set: { [weak self] isPresented in
                guard !isPresented, let self, let observedPresentationID else { return }
                guard
                    self.currentPresentation?.id == observedPresentationID,
                    let presentation = self.currentPresentation?.item,
                    matches(presentation)
                else { return }

                self.dismissCurrentPresentation()
            }
        )
    }

    func presentedItem(
        where matches: @escaping (Presentation) -> Bool
    ) -> Binding<TaggedItem<Presentation>?> {
        let observedPresentationID = currentPresentation.flatMap { presentation in
            matches(presentation.item) ? presentation.id : nil
        }

        return Binding(
            get: { [weak self] in
                guard
                    let presentation = self?.currentPresentation,
                    matches(presentation.item)
                else { return nil }

                return presentation
            },
            set: { [weak self] newPresentation in
                guard let self else { return }

                if let newPresentation {
                    guard newPresentation.id != self.currentPresentation?.id else { return }

                    self.present(newPresentation.item)
                    return
                }

                guard let observedPresentationID else { return }
                guard
                    self.currentPresentation?.id == observedPresentationID,
                    let presentation = self.currentPresentation?.item,
                    matches(presentation)
                else { return }

                self.dismissCurrentPresentation()
            }
        )
    }

    private func beginWaitingForPresenterReadiness() {
        readinessRequestID = UUID()
    }

    private func presentQueuedPresentation() {
        guard let queuedPresentation else { return }

        currentPresentation = TaggedItem(queuedPresentation)
        self.queuedPresentation = nil
    }
}

extension View {
    func presentationTransitionHost(
        _ coordinator: PresentationTransitionCoordinator<some Any>
    ) -> some View {
        presentationTransitionHost(
            state: coordinator.hostState,
            presenterDidBecomeReady: { requestID in
                coordinator.presenterDidBecomeReady(requestID)
            },
            hostDidDisappear: coordinator.hostDidDisappear
        )
    }

    func presentationTransitionHost(
        state: PresentationTransitionHostState,
        presenterDidBecomeReady: @escaping @MainActor (UUID) -> Void,
        hostDidDisappear: @escaping @MainActor () -> Void
    ) -> some View {
        background {
            PresenterReadinessObserver(
                state: state,
                onReady: presenterDidBecomeReady,
                onHostUnavailable: hostDidDisappear
            )
            .frame(width: 0, height: 0)
        }
    }
}

private struct PresenterReadinessObserver: UIViewControllerRepresentable {
    let state: PresentationTransitionHostState
    let onReady: @MainActor (UUID) -> Void
    let onHostUnavailable: @MainActor () -> Void

    func makeUIViewController(context _: Context) -> PresenterReadinessViewController {
        PresenterReadinessViewController()
    }

    func updateUIViewController(
        _ viewController: PresenterReadinessViewController,
        context _: Context
    ) {
        viewController.update(
            state: state,
            onReady: onReady,
            onHostUnavailable: onHostUnavailable
        )
    }

    static func dismantleUIViewController(
        _ viewController: PresenterReadinessViewController,
        coordinator _: Void
    ) {
        viewController.cancel()
    }
}

private final class PresenterReadinessTrackingView: UIView {
    var visibilityChanged: ((Bool) -> Void)?

    override func didMoveToWindow() {
        super.didMoveToWindow()
        visibilityChanged?(window != nil)
    }
}

private final class PresenterReadinessViewController: UIViewController {
    private var displayLink: CADisplayLink?
    private var state = PresentationTransitionHostState.idle
    private var onReady: (@MainActor (UUID) -> Void)?
    private var onHostUnavailable: (@MainActor () -> Void)?
    private var hasBeenVisible = false
    private var hasEverBeenVisible = false
    private var didReportHostUnavailable = false

    override func loadView() {
        let trackingView = PresenterReadinessTrackingView()
        trackingView.visibilityChanged = { [weak self] isVisible in
            self?.hostVisibilityChanged(isVisible)
        }
        view = trackingView
    }

    func update(
        state: PresentationTransitionHostState,
        onReady: @escaping @MainActor (UUID) -> Void,
        onHostUnavailable: @escaping @MainActor () -> Void
    ) {
        let stateChanged = self.state != state
        self.state = state
        self.onReady = onReady
        self.onHostUnavailable = onHostUnavailable

        if stateChanged {
            stopMonitoring()
        }

        startMonitoringIfNeeded()
    }

    func cancel() {
        stopMonitoring()
        reportHostUnavailableIfNeeded()

        hasBeenVisible = false
        state = .idle
        onReady = nil
        onHostUnavailable = nil
    }

    @objc private func inspectPresenter() {
        guard case let .awaitingPresenterReadiness(requestID) = state else {
            stopMonitoring()
            return
        }
        guard isPresenterReady else { return }

        let onReady = onReady
        stopMonitoring()
        onReady?(requestID)
    }

    private var isPresenterReady: Bool {
        var controller: UIViewController? = self

        while let current = controller {
            if current.presentedViewController != nil || current.transitionCoordinator != nil {
                return false
            }

            controller = current.parent
        }

        return viewIfLoaded?.window != nil
    }

    private func hostVisibilityChanged(_ isVisible: Bool) {
        if isVisible {
            hasBeenVisible = true
            hasEverBeenVisible = true
            didReportHostUnavailable = false
            startMonitoringIfNeeded()
            return
        }

        stopMonitoring()
        guard hasBeenVisible else { return }

        hasBeenVisible = false

        // full-screen presentations can detach their presenting view without ending host ownership
        guard isHiddenByPresentationContainer else { return }

        reportHostUnavailableIfNeeded()
    }

    private var isHiddenByPresentationContainer: Bool {
        // full-screen presentations detach their presenting view without ending host ownership
        guard presentedViewController == nil else { return false }

        var child: UIViewController = self

        while let parent = child.parent {
            if let navigationController = parent as? UINavigationController {
                return navigationController.visibleViewController !== child
            }

            if let tabBarController = parent as? UITabBarController {
                return tabBarController.selectedViewController !== child
            }

            child = parent
        }

        return false
    }

    private func reportHostUnavailableIfNeeded() {
        guard hasEverBeenVisible, !didReportHostUnavailable else { return }

        didReportHostUnavailable = true
        onHostUnavailable?()
    }

    private func startMonitoringIfNeeded() {
        guard displayLink == nil, viewIfLoaded?.window != nil else { return }
        guard case .awaitingPresenterReadiness = state else { return }

        // poll actual UIKit ownership because SwiftUI clears its binding before dismissal completes
        let displayLink = CADisplayLink(target: self, selector: #selector(inspectPresenter))
        displayLink.preferredFrameRateRange = CAFrameRateRange(
            minimum: 4,
            maximum: 10,
            preferred: 5
        )

        displayLink.add(to: .main, forMode: .common)
        self.displayLink = displayLink
    }

    private func stopMonitoring() {
        displayLink?.invalidate()
        displayLink = nil
    }
}
