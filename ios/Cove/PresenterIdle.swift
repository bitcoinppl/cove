import Foundation
import UIKit

/// Waits for the UIKit presenter chain to settle before work that needs a free presenter
///
/// SwiftUI alerts and confirmation dialogs are `UIAlertController`s that keep running their
/// button action while still on screen. Starting a passkey request or presenting a share sheet
/// in that window either cancels the ASAuthorization assertion silently or lands the new
/// controller on the dismissing dialog. An ordinary presented sheet does not count as busy, so
/// work that legitimately runs from inside a sheet is not delayed
enum PresenterIdle {
    private static let pollInterval = Duration.milliseconds(50)
    private static let timeout = Duration.milliseconds(1500)

    /// Counts consecutive idle checks against a deadline
    ///
    /// A single idle check can land between a dismissal ending and the next presentation
    /// starting, so the chain must read idle twice in a row before it counts as settled
    struct Settle {
        enum Step: Equatable {
            case settled
            case poll
            case timedOut
        }

        private static let requiredConsecutiveIdleChecks = 2

        private let deadline: ContinuousClock.Instant
        private var consecutiveIdleChecks = 0

        init(deadline: ContinuousClock.Instant) {
            self.deadline = deadline
        }

        mutating func record(idle: Bool, at now: ContinuousClock.Instant) -> Step {
            consecutiveIdleChecks = idle ? consecutiveIdleChecks + 1 : 0
            if consecutiveIdleChecks >= Self.requiredConsecutiveIdleChecks { return .settled }

            return now < deadline ? .poll : .timedOut
        }
    }

    /// Returns whether no controller from `rootViewController` down its presented chain is busy
    @MainActor
    static func isIdle(from rootViewController: UIViewController?) -> Bool {
        var current = rootViewController

        while let controller = current {
            if controller.isBeingPresented
                || controller.isBeingDismissed
                || controller.transitionCoordinator != nil
                || controller is UIAlertController
            {
                return false
            }

            current = controller.presentedViewController
        }

        return true
    }

    /// Suspends until the presenter chain is idle on consecutive checks, or gives up after the timeout
    ///
    /// Proceeds anyway on timeout so a stuck presenter cannot block the caller forever
    @MainActor
    static func wait() async {
        let clock = ContinuousClock()
        var settle = Settle(deadline: clock.now + timeout)

        while true {
            switch settle.record(idle: isIdle(from: foregroundRootViewController()), at: clock.now) {
            case .settled:
                return

            case .timedOut:
                Log.warn("[PRESENTER] still busy after \(timeout), proceeding anyway")
                return

            case .poll:
                do {
                    try await Task.sleep(for: pollInterval)
                } catch {
                    return
                }
            }
        }
    }

    /// Blocks a background thread until the presenter chain is idle, or gives up after the timeout
    ///
    /// Call this before any request timeout or semaphore starts so the wait does not count
    /// against it
    static func waitBlocking() {
        precondition(!Thread.isMainThread, "PresenterIdle.waitBlocking must not run on the main thread")

        let settled = DispatchSemaphore(value: 0)
        Task { @MainActor in
            await wait()
            settled.signal()
        }

        settled.wait()
    }

    @MainActor
    private static func foregroundRootViewController() -> UIViewController? {
        let scene = UIApplication.shared.connectedScenes
            .compactMap { $0 as? UIWindowScene }
            .first { $0.activationState == .foregroundActive }

        let window = scene?.windows.first { $0.isKeyWindow && !$0.isHidden }
            ?? scene?.windows.first { !$0.isHidden && $0.windowLevel == .normal }

        return window?.rootViewController
    }
}
