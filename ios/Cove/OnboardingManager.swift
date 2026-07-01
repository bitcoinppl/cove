import Foundation
import Observation

extension WeakReconciler: OnboardingManagerReconciler where Reconciler == OnboardingManager {}

@Observable
final class OnboardingManager: AnyReconciler, OnboardingManagerReconciler, @unchecked Sendable {
    @ObservationIgnored let rust: RustOnboardingManager
    @ObservationIgnored private let rustBridge = DispatchQueue(
        label: "cove.onboarding.rustbridge", qos: .userInitiated
    )
    let app: AppManager
    var state: OnboardingState
    var isComplete = false

    typealias Message = OnboardingReconcileMessage

    init(app: AppManager) {
        self.app = app
        let rust = RustOnboardingManager()
        self.rust = rust
        self.state = rust.state()
        rust.listenForUpdates(reconciler: WeakReconciler(self))
    }

    func dispatch(_ action: OnboardingAction) {
        rustBridge.async { [rust] in
            rust.dispatch(action: action)
        }
    }

    func reconcile(message: OnboardingReconcileMessage) {
        DispatchQueue.main.async { [weak self] in
            guard let self else { return }
            switch message {
            case let .step(step):
                state.step = step
            case let .branch(branch):
                state.branch = branch
            case let .createdWords(words):
                state.createdWords = words
            case let .cloudBackupEnabled(enabled):
                state.cloudBackupEnabled = enabled
            case let .secretWordsSaved(saved):
                state.secretWordsSaved = saved
            case let .cloudRestoreState(cloudRestoreState):
                state.cloudRestoreState = cloudRestoreState
            case let .cloudRestoreIssueChanged(cloudRestoreIssue):
                state.cloudRestoreIssue = cloudRestoreIssue
            case let .cloudRestoreProviderHintChanged(cloudRestoreProviderHint):
                state.cloudRestoreProviderHint = cloudRestoreProviderHint
            case let .shouldOfferCloudRestore(shouldOfferCloudRestore):
                state.shouldOfferCloudRestore = shouldOfferCloudRestore
            case let .cloudRestoreAlertVisible(cloudRestoreAlertVisible):
                state.cloudRestoreAlertVisible = cloudRestoreAlertVisible
            case let .restoreStateChanged(restoreState):
                state.restoreState = restoreState
            case let .errorChanged(error):
                state.error = error
            case .complete:
                isComplete = true
            }
        }
    }

    func reconcileMany(messages: [OnboardingReconcileMessage]) {
        messages.forEach { reconcile(message: $0) }
    }
}
