import Foundation
import UIKit

/// Waits for a ready presenter before work that needs one
///
/// A presenter is ready when the app lock is down, a foreground-active scene has a visible key
/// window, and no controller in that window's presented chain is busy. SwiftUI alerts and confirmation dialogs are `UIAlertController`s that keep running their
/// button action while still on screen. Starting a passkey request or presenting a share sheet
/// in that window either cancels the ASAuthorization assertion silently or lands the new
/// controller on the dismissing dialog. An ordinary presented sheet does not count as busy, so
/// work that legitimately runs from inside a sheet is not delayed
enum PresenterIdle {
    private static let pollInterval = Duration.milliseconds(50)

    /// How long a request waits for a ready presenter, matching Android's
    /// `ForegroundUiBridge.PRESENTER_READINESS_TIMEOUT_MS`
    static let readinessTimeout = Duration.milliseconds(1500)

    /// Whether a request can present UI now
    ///
    /// A ready presenter carries the window the request anchors to, so a passkey request cannot
    /// start without one
    enum Readiness {
        case ready(UIWindow)
        case notReady
    }

    /// Counts consecutive ready checks against a deadline
    ///
    /// A single check can land between a dismissal ending and the next presentation starting, so
    /// the presenter must read ready twice in a row before it counts as settled
    struct Settle<Ready> {
        enum Step {
            case settled(Ready)
            case poll
            case timedOut
        }

        private static var requiredConsecutiveReadyChecks: Int { 2 }

        private let deadline: ContinuousClock.Instant
        private var consecutiveReadyChecks = 0

        init(deadline: ContinuousClock.Instant) {
            self.deadline = deadline
        }

        mutating func record(_ ready: Ready?, at now: ContinuousClock.Instant) -> Step {
            guard let ready else {
                consecutiveReadyChecks = 0
                return now < deadline ? .poll : .timedOut
            }

            consecutiveReadyChecks += 1
            if consecutiveReadyChecks >= Self.requiredConsecutiveReadyChecks { return .settled(ready) }

            return now < deadline ? .poll : .timedOut
        }
    }

    /// Whether the app lock is up, from the `AuthManager` mirror of Rust's lock that also draws it
    ///
    /// The lock is a SwiftUI overlay inside an active, idle window, so the scene and controller
    /// facts alone would count a locked app as ready
    @MainActor
    static func isAppLocked() -> Bool {
        AuthManager.shared.lockState == .locked
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

    /// The visible key window of a foreground-active scene, the only window a request may anchor to
    ///
    /// A foreground-inactive scene (app switcher, Control Center, the app lock's Face ID prompt)
    /// or one without a key window cannot show the passkey sheet, so it has no ready window
    @MainActor
    static func readyWindow(in scenes: [any PresenterScene]) -> UIWindow? {
        for scene in scenes where scene.activationState == .foregroundActive {
            if let window = scene.windows.first(where: { $0.isKeyWindow && !$0.isHidden }) {
                return window
            }
        }

        return nil
    }

    /// The ready window if its presented chain is idle on this one check
    @MainActor
    private static func idleReadyWindow(in scenes: [any PresenterScene]) -> UIWindow? {
        readyWindow(in: scenes).flatMap { isIdle(from: $0.rootViewController) ? $0 : nil }
    }

    /// Suspends until a ready window's presenter chain is idle on consecutive checks
    ///
    /// Returns `.notReady` after the timeout, so a request never starts over a busy, inactive or
    /// missing presenter. A locked app is `.notReady` at once instead: only the user's credential
    /// opens the lock, and that is not the tap that started the request, so the request must not
    /// wait for it
    @MainActor
    static func waitUntilReady(
        timeout: Duration = readinessTimeout,
        scenes: @MainActor () -> [any PresenterScene] = connectedScenes,
        isAppLocked: @MainActor () -> Bool = isAppLocked
    ) async -> Readiness {
        let clock = ContinuousClock()
        var settle = Settle<UIWindow>(deadline: clock.now + timeout)

        while true {
            if isAppLocked() {
                Log.info("[PRESENTER] not ready: the app is locked")
                return .notReady
            }

            switch settle.record(idleReadyWindow(in: scenes()), at: clock.now) {
            case let .settled(window):
                return .ready(window)

            case .timedOut:
                Log.warn("[PRESENTER] not ready after \(timeout)")
                return .notReady

            case .poll:
                do {
                    try await Task.sleep(for: pollInterval)
                } catch {
                    return .notReady
                }
            }
        }
    }

    /// Whether `window`, which a request settled on, can still present now
    ///
    /// The caller blocks on the main queue between settling and starting its request, and the app
    /// can lock, leave the foreground or present an alert in that wait, so the request repeats
    /// every readiness fact on the main queue in the turn that starts it
    @MainActor
    static func isStillReady(
        _ window: UIWindow,
        scenes: @MainActor () -> [any PresenterScene] = connectedScenes,
        isAppLocked: @MainActor () -> Bool = isAppLocked
    ) -> Bool {
        guard !isAppLocked() else { return false }
        return idleReadyWindow(in: scenes()) === window
    }

    /// Suspends until the presenter is ready, or gives up after the timeout (at once while the app
    /// is locked) and lets the caller present anyway
    ///
    /// For share sheets and alerts, which UIKit still shows over a late presenter. Passkey
    /// requests use `waitUntilReadyBlocking` and stop when the presenter is not ready
    @MainActor
    static func wait() async {
        _ = await waitUntilReady()
    }

    /// Blocks a background thread until the presenter is ready or the timeout passes
    ///
    /// Call this before any request timeout or semaphore starts so the wait does not count
    /// against it
    static func waitUntilReadyBlocking() -> Readiness {
        precondition(!Thread.isMainThread, "PresenterIdle.waitUntilReadyBlocking must not run on the main thread")

        let result = ReadinessResult()
        let settled = DispatchSemaphore(value: 0)
        Task { @MainActor in
            result.readiness = await waitUntilReady()
            settled.signal()
        }

        settled.wait()
        return result.readiness
    }

    @MainActor
    static func connectedScenes() -> [any PresenterScene] {
        UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }
    }
}

extension PresenterIdle.Settle.Step: Equatable where Ready: Equatable {}

/// The scene state `PresenterIdle` reads, so tests can supply scenes UIKit would not create
@MainActor
protocol PresenterScene {
    var activationState: UIScene.ActivationState { get }
    var windows: [UIWindow] { get }
}

extension UIWindowScene: PresenterScene {}

/// Carries the readiness from the main actor back to the blocked thread, which reads it only after
/// the semaphore signals
private final class ReadinessResult: @unchecked Sendable {
    var readiness = PresenterIdle.Readiness.notReady
}
