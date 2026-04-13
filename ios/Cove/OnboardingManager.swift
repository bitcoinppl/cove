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
    var cloudCheckWarning: String?

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
                applyStep(step)
            case let .branch(branch):
                state.branch = branch
            case let .hardwareDevice(device):
                state.hardwareDevice = device
            case let .createdWords(words):
                state.createdWords = words
            case let .cloudBackupEnabled(enabled):
                state.cloudBackupEnabled = enabled
            case let .secretWordsSaved(saved):
                state.secretWordsSaved = saved
            case let .errorMessageChanged(errorMessage):
                state.errorMessage = errorMessage
            case .complete:
                isComplete = true
            }
        }
    }

    func reconcileMany(messages: [OnboardingReconcileMessage]) {
        messages.forEach { reconcile(message: $0) }
    }

    private func applyStep(_ step: OnboardingStep) {
        if state.step == .cloudCheck, step == .restoreOffer {
            cloudCheckWarning = state.errorMessage
        } else if step != .restoreOffer {
            cloudCheckWarning = nil
        }

        state.step = step
    }
}
